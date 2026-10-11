"""L3 generator: tuya-device-handlers quirk registry -> JSON (data only).

Usage: python gen_quirks.py <out.json>; then python build_rules.py --quirks <out.json>

Needs tuya-device-handlers (pinned) importable. Unknown `apply_when` callables / type-information
classes fail LOUDLY so new handler releases cannot silently drift from the engine."""
import json
import pathlib
import sys

import tuya_device_handlers
from tuya_device_handlers import TUYA_QUIRKS_REGISTRY as REG
from tuya_device_handlers.devices import register_tuya_quirks

# hand-mapped code hooks (spec: "3 code hooks"): callable name -> Cond over Status
WHEN = {"_is_fahrenheit_variant": {"and": [{"is_int": {"status": "temp_set"}}, {"ge": [{"status": "temp_set"}, 450]}]}}
TYPE_OVERRIDES = {"InvertedIntegerTypeInformationEx": "invert_int_max"}

def when(e):
    if e.apply_when is None:
        return None
    if e.apply_when.__name__ not in WHEN:
        raise SystemExit(f"unmapped apply_when {e.apply_when.__name__} — add it to WHEN")
    return WHEN[e.apply_when.__name__]

def mode(m):
    return ("R" if int(m) & 1 else "") + ("W" if int(m) & 2 else "")

def gen():
    register_tuya_quirks(None)
    out = {}
    for pid, q in sorted(REG._quirks.items()):
        ops = []
        if q._override_category:
            ops.append({"op": "SetCategory", "category": q._override_category})
        for e in q._quirk_entries:
            t = type(e).__name__
            base = {"dpid": e.dpid, "code": e.dpcode}
            w = when(e)
            if t == "_DatapointDefinition":
                d = {"op": "DefineDp", **base, "type": e.dptype.value, "mode": mode(e.dpmode),
                     "values": e.values, "report_type": e.report_type}
            elif t == "_DatapointRemoval":
                d = {"op": "RemoveDp", **base}
            elif t == "_InitialStatusValueMapping":
                d = {"op": "MapInitialStatus", **base, "mapping": [[k, v] for k, v in e.status_mapping.items()]}
            elif t == "_LocalConvertStrategy":
                d = {"op": "LocalConvert", **base, "value_convert": e.value_convert, "enum_mapping_map": e.enum_mapping_map}
            else:
                raise SystemExit(f"unknown quirk entry {t}")
            if w:
                d["when"] = w
            ops.append(d)
        for (dpid, code), cls in q._type_information_overrides.items():
            if cls.__name__ not in TYPE_OVERRIDES:
                raise SystemExit(f"unmapped type override {cls.__name__}")
            ops.append({"op": "TypeOverride", "dpid": dpid, "code": code, "as": TYPE_OVERRIDES[cls.__name__]})
        out[pid] = {"meta": {"manufacturer": q.manufacturer, "model": q.model, "model_id": q.model_id},
                    "ops": ops, "feeder_schedules": bool(q._get_wrapper_functions)}
    return {"handlers_version": getattr(tuya_device_handlers, "__version__", "0.0.29"), "quirks": out}

if __name__ == "__main__":
    pathlib.Path(sys.argv[1]).write_text(json.dumps(gen(), indent=1, ensure_ascii=False, default=str))
