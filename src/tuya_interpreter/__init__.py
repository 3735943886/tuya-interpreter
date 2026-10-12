"""Transport-independent Tuya ↔ ildevice interpretation, backed by the Rust engine."""
from .checks import Rejected, check_command
from .driver import HostEnv, TuyaDriver, default_env, descriptor_of
from .io import (
    Absent,
    CancelTimer,
    Command,
    Connected,
    Descriptor,
    Disconnected,
    Event,
    Message,
    Reject,
    SendMessage,
    SetTimer,
    SettingsChanged,
    Timer,
    Value,
)
from .native import merge_rules
from .overrides import OverrideError, merge_all
from .rules import load_builtin_rules


def preload() -> None:
    """Load the native library ahead of use; callers supply rule data explicitly."""
    from .native import preload as native_preload

    native_preload()


__all__ = [
    "Absent",
    "CancelTimer",
    "Command",
    "Connected",
    "Descriptor",
    "Disconnected",
    "Event",
    "HostEnv",
    "Message",
    "OverrideError",
    "Reject",
    "Rejected",
    "SendMessage",
    "SetTimer",
    "SettingsChanged",
    "Timer",
    "TuyaDriver",
    "Value",
    "check_command",
    "default_env",
    "descriptor_of",
    "load_builtin_rules",
    "merge_all",
    "merge_rules",
    "preload",
]
