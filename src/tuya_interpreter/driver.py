"""One Tuya device, driven by the Rust engine. The driver's whole state is the JSON the engine returns."""
from __future__ import annotations

import weakref
from dataclasses import asdict, dataclass, field

from . import io
from .native import _call, close_driver, register_rules
from .overrides import OverrideError


@dataclass
class HostEnv:
    """The host's unit system: its temperature unit and, per platform and device class, the units it accepts."""
    temperature_unit: str = "C"
    allowed_units: dict[str, dict[str, list[str]]] = field(default_factory=dict)


def default_env(rules: dict) -> HostEnv:
    return HostEnv(allowed_units=rules["host_units"])


def descriptor_of(device: dict, **kwargs) -> dict:
    driver = TuyaDriver(device, **kwargs)
    try:
        return driver.descriptor
    finally:
        driver.close()


class TuyaDriver:
    """A device driver whose state stays in the engine; each input sends only itself and its outputs."""

    def __init__(self, device: dict, *, rules: dict, env: HostEnv | None = None, use_quirks: bool = True, dpmap: dict | None = None,
                 allow_hazardous: bool = False, expose_unused: bool = False, overrides: dict | None = None,
                 device_settings: bool = False):
        self.rules_id = register_rules(rules)
        options = {"env": asdict(env or default_env(rules)), "use_quirks": use_quirks,
                   "dpmap": {str(k): v for k, v in (dpmap or {}).items()}, "allow_hazardous": allow_hazardous,
                   "expose_unused": expose_unused, "overrides": overrides or {}, "device_settings": device_settings}
        try:
            self._handle = self.call("open", device=device, options=options)["handle"]
        except ValueError as error:
            if str(error).startswith("override:"):
                raise OverrideError(str(error)) from error
            raise
        self._finalizer = weakref.finalize(self, close_driver, self._handle)
        self._status = {"linked": False, "synced": False, "timers": []}

    def call(self, op: str, **payload):
        """An engine operation against this driver's rule pack."""
        return _call({"op": op, "rules_id": self.rules_id, **payload})

    def _run(self, inp: dict, now: float) -> list[io.Output]:
        result = _call({"op": "drive", "handle": self._handle, "input": inp, "now": now})
        outputs = result.pop("outputs")
        self._status = result
        return [io.OUTPUTS[o.pop("type")](**o) for o in outputs]

    def handle(self, now: float, inp: io.Input) -> list[io.Output]:
        return self._run(io.wire(inp), now)

    def describe(self, seed=(), *, now: float) -> list[io.Output]:
        return self._run({"type": "describe", "seed": [io.wire(i) for i in seed]}, now)

    def carry(self, old: TuyaDriver) -> None:
        _call({"op": "carry_handle", "handle": self._handle, "old": old._handle})

    def settings_block(self) -> dict:
        return self.call("settings", driver=self.state)

    def close(self) -> None:
        """Release the engine's copy now; otherwise it is released when the driver is collected."""
        self._finalizer()

    @property
    def state(self) -> dict:
        """A version-specific snapshot of the whole driver, for inspection and migration."""
        return _call({"op": "snapshot", "handle": self._handle})

    def _view(self) -> dict:
        return _call({"op": "view", "handle": self._handle})

    @property
    def descriptor(self) -> dict:
        return self._view()["descriptor"]

    @property
    def linked(self) -> bool:
        return self._status["linked"]

    @property
    def synced(self) -> bool:
        return self._status["synced"]

    @property
    def timers(self) -> set[str]:
        return set(self._status["timers"])

    @property
    def dps(self) -> dict:
        return self._view()["dps"]
