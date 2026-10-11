"""Driver inputs and outputs (il.md section 8) as plain data. No behaviour, no I/O."""
from __future__ import annotations

import re
from dataclasses import asdict, dataclass
from typing import Any


# --- inputs -------------------------------------------------------------------
@dataclass(frozen=True)
class Connected:
    """The device link came up."""


@dataclass(frozen=True)
class Disconnected:
    """The device link dropped."""


@dataclass(frozen=True)
class Message:
    """A structured packet from the device. For Tuya, `channel` is one of

    * ``"active"``: device-initiated push (button, toggle, add_ele delta, event). Only this channel fires events
      and accumulates deltas.
    * ``"passive"`` / ``"state"``: readback, periodic report or full snapshot. Updates values only.

    `json` is a decoded flat DP map, e.g. ``{"1": true}``. The host unwraps any transport envelope.
    """
    channel: str
    json: Any


@dataclass(frozen=True)
class Command:
    """A consumer wants to write a property. The driver runs the il.md section 5 checks itself."""
    prop: str
    value: Any = None


@dataclass(frozen=True)
class Timer:
    name: str


# --- outputs ------------------------------------------------------------------
@dataclass(frozen=True)
class Descriptor:
    desc: dict


@dataclass(frozen=True)
class Value:
    prop: str
    value: Any


@dataclass(frozen=True)
class Absent:
    prop: str


@dataclass(frozen=True)
class Event:
    prop: str
    kind: str


@dataclass(frozen=True)
class SendMessage:
    """Write to the device. Channel ``"set"``: json ``{"dps": {"<dp id>": raw, ...}}``; the host adds the device."""
    channel: str
    json: Any


@dataclass(frozen=True)
class SetTimer:
    """Call the driver back with ``Timer(name)`` after `after` seconds (replaces a timer of the same name)."""
    name: str
    after: float


@dataclass(frozen=True)
class CancelTimer:
    name: str


@dataclass(frozen=True)
class Reject:
    prop: str
    code: str
    reason: str = ""


@dataclass(frozen=True)
class SettingsChanged:
    """A setting of the device's own was written (`TuyaDriver(device_settings=True)`): `block` is the device's whole
    settings block now (e.g. ``{"cover": {"invert_position": true}}``; empty: none), for the host to keep as the
    device's override block. It takes effect when the host reloads the overrides with it."""
    block: dict


Input = Connected | Disconnected | Message | Command | Timer
Output = Descriptor | Value | Absent | Event | SendMessage | SetTimer | CancelTimer | Reject | SettingsChanged


def tag(cls: type) -> str:
    """The engine's name for an input or output type (`SetTimer` -> ``set_timer``)."""
    return re.sub(r"(?<!^)(?=[A-Z])", "_", cls.__name__).lower()


def wire(item: Input | Output) -> dict:
    """An input or output as the engine's JSON."""
    return {"type": tag(type(item)), **asdict(item)}


OUTPUTS: dict[str, type] = {tag(c): c for c in (Descriptor, Value, Absent, Event, SendMessage, SetTimer, CancelTimer,
                                                  Reject, SettingsChanged)}
