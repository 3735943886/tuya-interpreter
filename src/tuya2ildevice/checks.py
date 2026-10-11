"""il.md section 5: check a write against the descriptor (the engine's `check`)."""
from __future__ import annotations

import math

from .native import call


class Rejected(Exception):
    def __init__(self, code: str, reason: str = ""):
        super().__init__(code, reason)
        self.code, self.reason = code, reason


def check_command(desc: dict, state: dict, prop: str, value):
    """The value to write, canonicalised; raises `Rejected`."""
    if isinstance(value, float) and not math.isfinite(value):
        value = None  # JSON has no non-finite numbers; both representations are invalid writes.
    result = call("check", descriptor=desc, state=state, prop=prop, value=value)
    if "reject" in result:
        raise Rejected(result["reject"], result["reason"])
    return result["accept"]
