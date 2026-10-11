"""Dump HA's per-device-class allowed units (HostEnv.allowed_units, spec 5.3). Needs `homeassistant`.

Usage: python gen_host_units.py <out.json>; then python build_rules.py --host-units <out.json>"""
import enum
import json
import sys

from homeassistant.components.number.const import DEVICE_CLASS_UNITS as N
from homeassistant.components.sensor.const import DEVICE_CLASS_UNITS as S


def conv(d):
    out = {}
    for dc, units in d.items():
        vals = []
        for u in units:
            if isinstance(u, type) and issubclass(u, enum.Enum): vals += [m.value for m in u]
            else: vals.append(u)
        out[dc.value] = sorted(vals, key=lambda x: (x is None, str(x)))
    return out
with open(sys.argv[1], "w", encoding="utf-8") as f:
    json.dump({"sensor": conv(S), "number": conv(N)}, f, indent=1, ensure_ascii=False)
