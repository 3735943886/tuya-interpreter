"""Sans-IO hub: one `TuyaDriver` per device behind one il-mqtt producer.

    device input (Connected/Disconnected/Message)   -->  Hub.on_bridge_message()  -->  il/<id>, ...
    bridge (set/get)  <--  BridgeCommand             <--  Hub.on_il()             <--  il/<id>/<prop>/set

Every decision (what a replaced device clears, when to ask the bridge for state, how values and writes are encoded)
is the engine's `hub` and `il` operations; this class keeps the drivers and turns the engine's actions into
`Publish`/`BridgeCommand`/`Schedule`/`Unschedule`/`SaveSettings` for the host to execute. Decoding the bridge's own
topics is the host's job (rustuya-local).
"""
from __future__ import annotations

from collections.abc import Sequence
from dataclasses import dataclass
from typing import Any

from .driver import TuyaDriver
from .io import Connected, Disconnected, Message, Timer, wire
from .native import _call

IL = "il"


@dataclass(frozen=True)
class Publish:
    side: str
    topic: str
    payload: str
    retain: bool = False
    qos: int = 1


@dataclass(frozen=True)
class Subscribe:
    side: str
    topic: str
    qos: int = 1


@dataclass(frozen=True)
class Schedule:
    """Ask the host to call `Hub.on_timer(now, device_id, name)` after `after` seconds (replacing an equal name)."""
    device_id: str
    name: str
    after: float


@dataclass(frozen=True)
class Unschedule:
    device_id: str
    name: str


@dataclass(frozen=True)
class BridgeCommand:
    """Ask the host to tell the bridge to read (`get`) or write (`set`, `dps`) a device."""
    device_id: str
    action: str
    dps: dict[str, Any] | None = None


@dataclass(frozen=True)
class SaveSettings:
    """Ask the host to keep `block` as the device's settings (its override block; empty removes it) and reload."""
    device_id: str
    block: dict


class IlTopics:
    """The il-mqtt topic layout: a prefix and this producer's `source`."""

    def __init__(self, prefix: str = "il", source: str = "tuya"):
        self.prefix, self.source = prefix, source

    def call(self, action: str, **fields: Any) -> Any:
        return _call({"op": "il", "topics": vars(self), "action": action, **fields})

    def descriptor(self, id: str) -> str:
        return self.call("topics", device=id)["descriptor"]

    def state(self, id: str, prop: str) -> str:
        return self.call("topics", device=id, prop=prop)["state"]

    def reject(self, id: str) -> str:
        return self.call("topics", device=id)["reject"]

    @property
    def presence(self) -> str:
        return self.call("topics")["presence"]

    def set_subscription(self) -> str:
        return self.call("topics")["set_subscription"]

    def parse_set(self, topic: str) -> tuple[str, str] | None:
        loc = self.call("parse_set", topic=topic)
        return tuple(loc) if loc else None


def encode_value(value: Any) -> str:
    """A state payload (il-mqtt.md section 3)."""
    return IlTopics().call("encode", value=value)


def decode_write(ptype: str, payload: str) -> Any:
    """A `set` payload as a command value; the driver's checks refuse what does not fit."""
    return IlTopics().call("decode", type=ptype, payload=payload)


_ACTIONS = {
    "publish": lambda i, a: Publish(IL, a["topic"], a["payload"], a["retain"], a["qos"]),
    "schedule": lambda i, a: Schedule(i, a["name"], a["after"]),
    "unschedule": lambda i, a: Unschedule(i, a["name"]),
    "bridge": lambda i, a: BridgeCommand(i, a["action"], a["dps"]),
    "save_settings": lambda i, a: SaveSettings(i, a["block"]),
}


def _actions(device_id: str, actions: list[dict]) -> list:
    return [_ACTIONS[a["kind"]](device_id, a) for a in actions]


class Hub:
    """One driver per Tuya device id. `devices`: tuyadevices.json entries. Hosts call `start()` once, then feed
    `on_bridge_message` / `on_il` / `on_timer` and execute what they return. `now` is whatever clock the host reads.
    `driver_kwargs` go to every `TuyaDriver` (`allow_hazardous`, `expose_unused`, `use_quirks`, `overrides`,
    `device_settings`, `rules`)."""

    def __init__(self, devices: list[dict], *, il: IlTopics | None = None, **driver_kwargs: Any):
        self.il = il or IlTopics()
        self._kw = driver_kwargs
        for d in devices:
            self._check_id(d["id"])
        self._devices = {d["id"]: d for d in devices}
        self.drivers = {i: TuyaDriver(d, **driver_kwargs) for i, d in self._devices.items()}

    @property
    def records(self) -> dict[str, dict]:
        """The device records the Hub drives now, by id."""
        return dict(self._devices)

    def _check_id(self, device_id: str) -> None:
        if not self.il.call("usable_id", device=device_id):
            raise ValueError(f"device id {device_id!r} is not usable as an il-mqtt topic level (M-2)")

    def _hub(self, owner: TuyaDriver, action: str, /, **fields: Any) -> dict:
        return owner.call("hub", action=action, topics=vars(self.il), **fields)

    def _adopt(self, old: TuyaDriver | None, new: TuyaDriver, **fields: Any) -> list:
        result = self._hub(new, "adopt", old=old.state if old else None, new=new.state, **fields)
        if not result["kept"]:
            new.state = result["driver"]
            self.drivers[new.state["device"]["id"]] = new
        return _actions(new.state["device"]["id"], result["actions"])

    def set_device(self, device: dict, seed: Sequence[Connected | Disconnected | Message] = (), now: float = 0) -> list:
        """Add a device, or replace the record of a known one. `seed`: what the host already knows of it (link state,
        then the bridge's `state` snapshot), applied before anything is published. Raises, changing nothing, if the
        record cannot be driven."""
        self._check_id(device["id"])
        new = TuyaDriver(device, **self._kw)
        pubs = self._adopt(self.drivers.get(device["id"]), new, seed=[wire(i) for i in seed], now=now)
        self._devices[device["id"]] = device
        return pubs

    def remove_device(self, device_id: str) -> list:
        """M-11: clear every retained value, then the descriptor. Unknown ids do nothing."""
        old = self.drivers.pop(device_id, None)
        self._devices.pop(device_id, None)
        return _actions(device_id, self._hub(old, "remove", old=old.state)["actions"]) if old else []

    def reload(self, overrides: dict | None, *, rules: dict | None = None) -> list:
        """Apply new overrides (and rules). A device whose driving changed gets a fresh driver, replayed from the dps
        the old one had. Raises `OverrideError` (changing nothing) if any device refuses them."""
        kw = {**self._kw, "overrides": overrides}
        if rules is not None:
            kw["rules"] = rules
        fresh = {i: TuyaDriver(d, **kw) for i, d in self._devices.items()}          # all or nothing
        pubs: list = []
        for i, new in fresh.items():
            pubs += self._adopt(self.drivers[i], new, reload=True)
        self._kw = kw
        return pubs

    def subscriptions(self) -> list[Subscribe]:
        return [Subscribe(IL, self.il.set_subscription())]

    def presence(self, online: bool) -> Publish:
        """M-12. Register `presence(False)` as the Last Will on the il connection."""
        (pub,) = _actions("", [self.il.call("presence", online=online)])
        return pub

    def start(self) -> list:
        """Presence, then every device's descriptor and `available: false` (retained)."""
        pubs: list = [self.presence(True)]
        for i in self.drivers:
            pubs += self._input(i, {"type": "describe", "seed": []})
        return pubs

    def stop(self) -> list[Publish]:
        return [self.presence(False)]

    def _input(self, device_id: str, inp: dict, now: float = 0, **fields: Any) -> list:
        d = self.drivers.get(device_id)
        if d is None:
            return []
        result = self._hub(d, "input", driver=d.state, input=inp, now=now, **fields)
        d.state = result["driver"]
        return _actions(device_id, result["actions"])

    def on_bridge_message(self, now: float, device_id: str, inp: Connected | Disconnected | Message,
                          retained: bool = False) -> list:
        """`inp` is already decoded from the bridge's topics (the host's job)."""
        return self._input(device_id, wire(inp), now, retained=retained)

    def on_timer(self, now: float, device_id: str, name: str) -> list:
        return self._input(device_id, wire(Timer(name)), now)

    def on_il(self, now: float, topic: str, payload: bytes | str, retained: bool = False) -> list:
        loc = self.il.parse_set(topic)
        if loc is None or retained:                              # M-9: never replay an old write
            return []
        d = self.drivers.get(loc[0])
        if d is None:
            return []
        if isinstance(payload, bytes):
            payload = payload.decode("utf-8", "replace")
        result = self._hub(d, "write", driver=d.state, prop=loc[1], payload=payload, now=now)
        d.state = result["driver"]
        return _actions(loc[0], result["actions"])


__all__ = ["IL", "BridgeCommand", "Hub", "IlTopics", "Publish", "SaveSettings", "Schedule", "Subscribe",
           "Unschedule", "decode_write", "encode_value"]
