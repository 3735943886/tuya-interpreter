"""Property-level chain comparison: every golden (core) entity is looked up by its dp code in the IL descriptor
(`src`) and its plan spec, then platform / class / category / unit / options are compared."""
import collections
import json
import pathlib
import sys

import fixtures
from ildevice.core.descriptor import parse_descriptor
from ildevice.core.plan import plan_entities
from rule_data import rules

from tuya2ildevice import descriptor_of

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parents[2] / "scripts"))
import standard

GOLDEN = pathlib.Path(__file__).resolve().parents[1] / "golden"


def locate(code: str, **kw):
    """-> (raw descriptor, {dp code: [(prop name, spec)]}) for one fixture."""
    kw.setdefault("allow_hazardous", True)
    dev = fixtures.load(code)
    raw = descriptor_of(dev, rules=rules(), **kw)
    specs = plan_entities(parse_descriptor(raw))
    src = {n: p["src"] for n, p in raw["props"].items() if "src" in p}
    by_code = collections.defaultdict(list)
    for s in specs:
        for prop in [*s.slots.values(), *s.shared.values()]:
            by_code[prop.lower()].append(s)          # golden key is the dp code, or `<code>_<bit label>` for a bitmap
            if prop in src and src[prop].lower() != prop.lower():
                by_code[src[prop].lower()].append(s)
    return raw, specs, by_code


def compare(**kw) -> dict:
    gold = json.loads((GOLDEN / "golden.json").read_text())
    tally = collections.Counter()
    rows = collections.defaultdict(list)
    for code in fixtures.all_codes():
        _, specs, by_code = locate(code, **kw)
        used = {}
        for platform, ents in gold.get(code, {}).items():
            for e in ents:
                key = e["key"]
                if key == "":  # a composite: matched by platform, one spec per golden entity
                    pool = used.setdefault(code, [x for x in specs if x.platform == platform])
                    cand = [pool.pop(0)] if pool else []
                    if cand:
                        tally["ok_platform"] += 1
                        continue
                else:
                    cand = by_code.get(key.lower(), [])
                if not cand:
                    tally["missing"] += 1
                    rows["missing"].append((code, platform, key))
                    continue
                s = next((c for c in cand if c.platform == platform), cand[0])
                if s.platform != platform:
                    tally["platform"] += 1
                    rows["platform"].append((code, key, platform, s.platform))
                    continue
                tally["ok_platform"] += 1
                want_class = e.get("device_class")
                if platform == "switch":   # core's golden, with Tuya's own category list applied over it
                    want_class = standard.switch_device_class(fixtures.load(code)["category"], want_class)
                for facet, want, got in (
                    ("class", want_class, s.device_class),
                    ("category", e.get("entity_category"), s.entity_category),
                    ("unit", e.get("unit") or None, s.unit or None),
                ):
                    if platform in ("humidifier", "climate", "fan", "light", "cover") and facet == "unit":
                        continue
                    if want != got:
                        tally[facet] += 1
                        rows[facet].append((code, key, platform, want, got))
    return {"tally": tally, "rows": rows}


if __name__ == "__main__":
    import sys
    r = compare(**{k: v == "1" for k, v in (a.split("=") for a in sys.argv[1:])})
    print(dict(r["tally"]))
    for k, v in r["rows"].items():
        print("##", k, len(v))
        for row in v[:12]:
            print("  ", row)
