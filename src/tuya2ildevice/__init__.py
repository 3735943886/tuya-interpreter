"""tuya2ildevice: the one place that interprets Tuya, sans-IO. A thin binding of the bundled Rust engine, which is
driven by the JSON rule pack (`rules/`).

* `TuyaDriver`: one device: descriptor, live dps packets -> values/events, ildevice commands -> dps.
* `Hub`: many devices behind one il-mqtt producer. Decoding the bridge's own topics, the MQTT connection, timers and
  files are the host's (rustuya-local).
"""
from .checks import Rejected, check_command
from .driver import HostEnv, TuyaDriver, default_env, descriptor_of
from .io import (
    Absent,
    Command,
    Connected,
    Descriptor,
    Disconnected,
    Event,
    Message,
    Reject,
    SendMessage,
    SettingsChanged,
    Timer,
    Value,
)
from .mqtt import (
    BridgeCommand,
    Hub,
    IlTopics,
    Publish,
    SaveSettings,
    Schedule,
    Subscribe,
    Unschedule,
)
from .overrides import OverrideError, merge_all


def preload() -> None:
    """Load the engine library and the rule pack, which are otherwise loaded on first use. Both block: a host whose
    event loop must not (Home Assistant) imports and calls this in a worker thread before it drives devices."""
    from .native import preload as native_preload

    native_preload()


__all__ = [
    "Absent",
    "BridgeCommand",
    "Command",
    "Connected",
    "Descriptor",
    "Disconnected",
    "Event",
    "HostEnv",
    "Hub",
    "IlTopics",
    "Message",
    "OverrideError",
    "Publish",
    "Reject",
    "Rejected",
    "SaveSettings",
    "Schedule",
    "SendMessage",
    "SettingsChanged",
    "Subscribe",
    "Timer",
    "TuyaDriver",
    "Unschedule",
    "Value",
    "check_command",
    "default_env",
    "descriptor_of",
    "merge_all",
    "preload",
]
