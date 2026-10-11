"""What the drivers publish fits their descriptors, over every core fixture: names are real names, and each value a
fixture's own state produces is within its property's range, one of its options, or a colour."""
import re

import fixtures
import pytest
from rule_data import rules

from tuya2ildevice import Connected, Message, TuyaDriver, Value

WIRE = {"Boolean", "Integer", "Enum", "String", "Raw", "Json", "Bitmap"}


def record(code):
    """The fixture with dps numbered in status order and a local_strategy, as a real device record has one."""
    d = fixtures.load(code)
    codes = list(d["status"])
    types = {}
    for c in codes:
        spec = d["status_range"].get(c) or d["function"].get(c)
        v = d["status"][c]
        types[c] = spec["type"] if spec and spec["type"] in WIRE else (
            "Boolean" if isinstance(v, bool) else "Integer" if isinstance(v, int) else "String")
    d["local_strategy"] = {str(i + 1): {"value_convert": "default", "status_code": c,
                                        "config_item": {"statusFormat": {c: "$"}, "valueType": types[c],
                                                        "valueDesc": {}, "enumMappingMap": {}}}
                           for i, c in enumerate(codes)}
    return d, {str(i + 1): d["status"][c] for i, c in enumerate(codes)}


def fits(spec, value):
    t = spec["type"]
    if t == "binary":
        return isinstance(value, bool)
    if t == "number":
        return isinstance(value, (int, float)) and spec.get("min", value) <= value <= spec.get("max", value)
    if t == "select":
        return value in spec["options"]
    if spec.get("role") == "color":
        return re.fullmatch(r"#[0-9a-f]{6}", value) is not None
    return True


@pytest.mark.parametrize("code", fixtures.all_codes())
def test_a_fixture_state_publishes_values_that_fit(code):
    rec, dps = record(code)
    d = TuyaDriver(rec, allow_hazardous=True, rules=rules())
    props = d.descriptor["props"]
    assert not [p for p in props if p == "prop" or p.startswith("prop_")]       # a composite without a core key
    out = d.handle(0, Connected()) + d.handle(1, Message("state", dps))
    bad = [(o.prop, o.value) for o in out if isinstance(o, Value) and not fits(props[o.prop], o.value)]
    assert not bad


def test_a_fan_and_a_climate_power_are_named_after_their_dp():
    fan, _ = record("fs_g0ewlb1vmwqljzji")
    assert TuyaDriver(fan, rules=rules()).descriptor["props"]["switch"]["role"] == "on"
    ac, _ = record("cs_b9oyi2yofflroq1g")
    assert TuyaDriver(ac, rules=rules()).descriptor["props"]["switch"]["role"] == "on"


@pytest.mark.parametrize("code,prop,unit,value", [
    ("cz_qxJSyTLEtX5WrzA9", "cur_current", "mA", 81),
    ("cz_2jxesipczks0kdct", "cur_current", "A", 0.083),
    ("dlq_z3jngbyubvwgfrcv", "phase_aelectriccurrent", "mA", 608),
    ("dlq_z3jngbyubvwgfrcv", "phase_apower", "W", 133),
])
def test_electricity_values_keep_their_native_units(code, prop, unit, value):
    rec, dps = record(code)
    driver = TuyaDriver(rec, rules=rules())
    assert driver.descriptor["props"][prop]["unit"] == unit
    out = driver.handle(0, Connected()) + driver.handle(1, Message("state", dps))
    published = {o.prop: o.value for o in out if isinstance(o, Value)}
    assert published[prop] == pytest.approx(value)


@pytest.mark.parametrize("code", fixtures.all_codes())
def test_numeric_properties_declare_the_unit_of_the_published_value(code):
    rec, _ = record(code)
    driver = TuyaDriver(rec, allow_hazardous=True, rules=rules())
    plans = driver.state["plans"]
    for prop, binding in driver.state["assembly"]["bindings"].items():
        plan = plans[binding["source"]["plan"]] if "plan" in binding["source"] else {}
        if plan.get("platform") not in ("sensor", "number"):
            continue
        spec = driver.descriptor["props"][prop]
        if spec["type"] == "number":
            native = plan["identity"].get("native_unit")
            assert spec.get("unit") == (native.replace("µ", "μ") if native else None)
