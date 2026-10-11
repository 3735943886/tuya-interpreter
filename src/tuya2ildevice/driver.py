"""One Tuya device, driven by the Rust engine. The driver's whole state is the JSON the engine returns."""
from __future__ import annotations

from dataclasses import asdict, dataclass, field

from . import io
from .native import _call, register_rules
from .overrides import OverrideError


@dataclass
class HostEnv:
    """The host's unit system: its temperature unit and, per platform and device class, the units it accepts."""
    temperature_unit: str = "C"
    allowed_units: dict[str, dict[str, list[str]]] = field(default_factory=dict)


def default_env(rules: dict) -> HostEnv:
    return HostEnv(allowed_units=rules["host_units"])


def descriptor_of(device: dict, **kwargs) -> dict:
    return TuyaDriver(device, **kwargs).descriptor


class TuyaDriver:
    def __init__(self, device: dict, *, rules: dict, env: HostEnv | None = None, use_quirks: bool = True, dpmap: dict | None = None,
                 allow_hazardous: bool = False, expose_unused: bool = False, overrides: dict | None = None,
                 device_settings: bool = False):
        pack = rules
        self.rules_id = register_rules(pack)
        options = {"env": asdict(env or default_env(pack)), "use_quirks": use_quirks,
                   "dpmap": {str(k): v for k, v in (dpmap or {}).items()}, "allow_hazardous": allow_hazardous,
                   "expose_unused": expose_unused, "overrides": overrides or {}, "device_settings": device_settings}
        try:
            result = self.call("create", device=device, options=options)
        except ValueError as error:
            if str(error).startswith("override:"):
                raise OverrideError(str(error)) from error
            raise
        self.state = result["driver"]

    def call(self, op: str, **payload):
        """An engine operation against this driver's rule pack."""
        return _call({"op": op, "rules_id": self.rules_id, **payload})

    def _run(self, inp: dict, now: float) -> list[io.Output]:
        result = self.call("handle", driver=self.state, input=inp, now=now)
        self.state = result["driver"]
        return [io.OUTPUTS[o.pop("type")](**o) for o in result["outputs"]]

    def handle(self, now: float, inp: io.Input) -> list[io.Output]:
        return self._run(io.wire(inp), now)

    def describe(self, seed=(), *, now: float) -> list[io.Output]:
        return self._run({"type": "describe", "seed": [io.wire(i) for i in seed]}, now)

    def carry(self, old: TuyaDriver) -> None:
        self.state = self.call("carry", driver=self.state, old=old.state)["driver"]

    def settings_block(self) -> dict:
        return self.call("settings", driver=self.state)

    @property
    def descriptor(self) -> dict:
        return self.state["assembly"]["descriptor"]

    @property
    def linked(self) -> bool:
        return self.state["linked"]

    @property
    def synced(self) -> bool:
        return self.state["synced"]

    @property
    def timers(self) -> set[str]:
        return set(self.state["timers"])

    @property
    def dps(self) -> dict:
        return dict(self.state["dps"])
