"""User overrides for non-standard devices: dp renames, `remap` (alias / invert), per-device `expose_unused`, named
converter types and properties defined from a dp."""
import json
import pathlib

import pytest
from helpers import curtain, fn, strat
from rule_data import rules
from test_driver import SCHEMA, needs_spec

from tuya_interpreter import Command, Connected, Message, SendMessage, TuyaDriver, Value
from tuya_interpreter.overrides import OverrideError

HERE = pathlib.Path(__file__).parent
PRODUCTS = json.loads((HERE / "sample_products.json").read_text())      # cloud schemas of a window opener and curtains
MINE = json.loads((HERE / "sample_overrides.json").read_text())          # one installation's overrides for them


def _values(outs):
    return {o.prop: o.value for o in outs if isinstance(o, Value)}


def _sent(outs):
    return [o.json["dps"] for o in outs if isinstance(o, SendMessage)]


def _odd_curtain():
    """A curtain whose control dp speaks on/off/pause and whose position runs the other way."""
    d = curtain()
    for t in (d["function"], d["status_range"]):
        t["control"] = fn("control", "Enum", range=["on", "pause", "off"])
    return d


# --- rename ------------------------------------------------------------------------------------
def test_a_code_only_dp_entry_renames_and_keeps_the_type():
    dev = curtain()
    dev["local_strategy"] = strat({"1": ("mach_ctrl", "Enum"), "2": ("percent_control", "Integer"),
                                   "3": ("percent_state", "Integer")})
    for t in (dev["function"], dev["status_range"]):
        t["mach_ctrl"] = t.pop("control")
        t["mach_ctrl"]["code"] = "mach_ctrl"
    assert "open" not in TuyaDriver(dev, rules=rules()).descriptor["props"]                   # the tables do not know mach_ctrl
    d = TuyaDriver(dev, overrides={"p": {"dp": {"1": {"code": "control"}}}}, rules=rules())
    assert d.descriptor["props"]["open"]["role"] == "open"
    d.handle(0, Connected())
    assert _sent(d.handle(1, Command("stop", None))) == [{"1": "stop"}]


def test_a_rename_of_a_dp_the_device_lacks_is_an_error():
    with pytest.raises(OverrideError, match="no dp 42"):
        TuyaDriver(curtain(), overrides={"p": {"dp": {"42": {"code": "control"}}}}, rules=rules())
    with pytest.raises(OverrideError, match="only a `code`"):
        TuyaDriver(curtain(), overrides={"p": {"dp": {"1": {"code": "control", "mode": "R"}}}}, rules=rules())


# --- remap -------------------------------------------------------------------------------------
def test_alias_translates_both_ways_and_the_enum_range():
    ov = {"p": {"remap": {"control": {"alias": {"on": "open", "off": "close", "pause": "stop"}}}}}
    assert "open" not in TuyaDriver(_odd_curtain(), rules=rules()).descriptor["props"]    # no standard word: no open/close/stop
    d = TuyaDriver(_odd_curtain(), overrides=ov, rules=rules())
    assert {"open", "close", "stop"} <= set(d.descriptor["props"])
    d.handle(0, Connected())
    assert _sent(d.handle(1, Command("stop", None))) == [{"1": "pause"}]
    d = TuyaDriver(_odd_curtain(), overrides={"p": {**ov["p"], "remove": ["percent_control"]}}, rules=rules())
    d.handle(0, Connected())                                # no set-position dp: open/close go to the control dp
    assert _sent(d.handle(2, Command("close", None))) == [{"1": "off"}]


def test_alias_is_applied_before_the_converters_see_the_value():
    ov = {"p": {"remap": {"control": {"alias": {"on": "open", "off": "close", "pause": "stop"}}},
                "cover": {"state_source": "inferred"}}}
    d = TuyaDriver(_odd_curtain(), overrides=ov, rules=rules())
    d.handle(0, Connected())
    d.handle(1, Message("state", {"3": 50}))
    assert _values(d.handle(2, Message("active", {"1": "on"})))["cover_state"] == "opening"


def test_invert_mirrors_an_integer_and_negates_a_boolean():
    ov = {"p": {"remap": {"percent_state": {"invert": True}, "percent_control": {"invert": True}}}}
    plain, inv = TuyaDriver(curtain(), rules=rules()), TuyaDriver(curtain(), overrides=ov, rules=rules())
    for d in (plain, inv):
        d.handle(0, Connected())
    assert _values(inv.handle(1, Message("active", {"3": 30})))["position"] == \
        100 - _values(plain.handle(1, Message("active", {"3": 30})))["position"]
    assert _sent(inv.handle(2, Command("position", 80))) == [{"2": 100 - _sent(plain.handle(2, Command("position", 80)))[0]["2"]}]

    from test_driver import _kg
    d = TuyaDriver(_kg(), overrides={"prod1": {"remap": {"switch_1": {"invert": True}}}}, rules=rules())
    d.handle(0, Connected())
    assert _values(d.handle(1, Message("active", {"1": True})))["switch_1"] is False
    assert _sent(d.handle(2, Command("switch_1", True))) == [{"1": False}]


def test_remap_errors_are_loud():
    for bad in ({"p": {"remap": {"control": {"alias": {"a": "open", "b": "open"}}}}},
                {"p": {"remap": {"control": {"invert": "yes"}}}},
                {"p": {"remap": {"control": {"words": {}}}}},
                {"p": {"expose_unused": "yes"}}):
        with pytest.raises(OverrideError):
            TuyaDriver(curtain(), overrides=bad, rules=rules())


# --- per-device expose_unused, converters ----------------------------------------------
def test_expose_unused_per_device():
    from test_driver import _kg
    names = set(TuyaDriver(_kg(), overrides={"prod1": {"expose_unused": True}}, rules=rules()).descriptor["props"])
    assert names == set(TuyaDriver(_kg(), expose_unused=True, rules=rules()).descriptor["props"])
    assert "cycle_time" in names
    assert "cycle_time" not in TuyaDriver(_kg(), expose_unused=True,
                                          overrides={"prod1": {"expose_unused": False}}, rules=rules()).descriptor["props"]


def test_a_converter_is_named_by_its_block_and_configured_by_it():
    with pytest.raises(OverrideError, match="unknown converter"):
        TuyaDriver(curtain(), overrides={"p": {"converters": {"doubler": {"factor": 3}}}}, rules=rules())
    double = {"props": {"double": {"type": "number"}}, "factor": 3,
              "update": [{"emit": "double", "value": {"op": "mul", "args": [
                  {"op": "get", "path": "codes.percent_state", "default": 0}, {"op": "get", "path": "config.factor"}]}}]}
    d = TuyaDriver(curtain(), overrides={"p": {"converters": {"declarative": double}}}, rules=rules())
    d.handle(0, Connected())
    assert _values(d.handle(1, Message("active", {"3": 10})))["double"] == 30


# --- a whole installation's overrides --------------------------------------------------------------
def test_position_reads_back_from_the_target_where_the_override_says_so():
    d = TuyaDriver(PRODUCTS["f6jujmx0is5td50x"], overrides=MINE, rules=rules())
    d.handle(0, Connected())
    plain = TuyaDriver(PRODUCTS["f6jujmx0is5td50x"], rules=rules())
    plain.handle(0, Connected())
    assert "position" not in _values(plain.handle(1, Message("active", {"2": 30})))     # read from percent_state
    assert "position" in _values(d.handle(1, Message("active", {"2": 30})))


@needs_spec
def test_overridden_descriptors_validate():
    jsonschema = pytest.importorskip("jsonschema")
    schema = json.loads(SCHEMA.read_text())
    for dev in PRODUCTS.values():
        jsonschema.validate(TuyaDriver(dev, overrides=MINE, rules=rules()).descriptor, schema)


# --- properties defined from a dp -----------------------------------------------------------------
WINDOW = {"5rta89nj": {
    "dp": {"104": {"code": "percent_control", "type": "Integer", "mode": "RW",
                   "values": {"unit": "%", "min": 0, "max": 100, "scale": 0, "step": 1}}},
    "device": {"kind": "cover", "class": "window"},
    "props": {"position": {"src": "percent_control", "role": "position"},
              "open": {"src": "percent_control", "type": "trigger", "role": "open", "send": 100},
              "close": {"src": "percent_control", "type": "trigger", "role": "close", "send": 0},
              "battery": {"src": "residual_electricity", "role": "battery", "category": "diagnostic"}}}}


def test_a_cover_from_a_position_dp_alone():
    d = TuyaDriver(PRODUCTS["5rta89nj"], overrides=WINDOW, rules=rules())
    desc = d.descriptor
    assert desc["kind"] == "cover" and desc["class"] == "window"
    assert desc["props"]["position"] == {"type": "number", "min": 0, "max": 100, "step": 1, "unit": "%", "rw": True,
                                         "role": "position", "src": "percent_control"}
    assert desc["props"]["open"] == {"type": "trigger", "role": "open", "src": "percent_control"}
    assert desc["props"]["battery"] == {"type": "number", "min": 0, "max": 100, "step": 1, "unit": "%", "role": "battery",
                                        "category": "diagnostic", "src": "residual_electricity"}
    d.handle(0, Connected())
    assert _values(d.handle(1, Message("active", {"104": 40, "4": 80}))) == {"available": True, "position": 40,
                                                                              "battery": 80}
    assert _sent(d.handle(2, Command("position", 70))) == [{"104": 70}]
    assert _sent(d.handle(3, Command("open", None))) == [{"104": 100}]
    assert _sent(d.handle(4, Command("close", None))) == [{"104": 0}]
    assert d.handle(5, Command("position", 101))[0].code == "out_of_range"


def test_a_defined_property_follows_remap_and_replaces_the_tables_one():
    ov = {"cur1": {"remap": {"percent_control": {"invert": True}},
                   "props": {"position": {"src": "percent_control", "role": "position", "label": "Target"}}}}
    d = TuyaDriver(curtain(), overrides=ov, rules=rules())
    assert d.descriptor["props"]["position"]["label"] == "Target" and d.descriptor["props"]["position"]["src"] == "percent_control"
    d.handle(0, Connected())
    assert _values(d.handle(1, Message("active", {"2": 30})))["position"] == 70
    assert _sent(d.handle(2, Command("position", 80))) == [{"2": 20}]


def test_auto_false_keeps_only_what_the_block_defines():
    ov = {"cur1": {"auto": False, "props": {"mode": {"src": "control", "rw": False}}}}
    d = TuyaDriver(curtain(), overrides=ov, rules=rules())
    assert set(d.descriptor["props"]) == {"available", "mode"} and "kind" not in d.descriptor
    assert d.descriptor["props"]["mode"]["type"] == "select" and "rw" not in d.descriptor["props"]["mode"]
    assert d.handle(0, Command("mode", "open"))[0].code == "read_only"


def test_definition_errors_are_loud():
    for props in ({"x": {"src": "nope"}},                                       # no such dp
                  {"x": {"src": "percent_control", "type": "trigger"}},         # a trigger needs send
                  {"x": {"src": "percent_control", "send": 1}},                 # send only on a trigger
                  {"x": {"src": "percent_control", "type": "binary"}},          # does not fit an Integer
                  {"available": {"src": "percent_control"}}):
        with pytest.raises(OverrideError):
            TuyaDriver(curtain(), overrides={"cur1": {"props": props}}, rules=rules())
    ro = {"x": {"src": "residual_electricity", "rw": True}}                    # status only: not writable
    with pytest.raises(OverrideError):
        TuyaDriver(PRODUCTS["5rta89nj"], overrides={"5rta89nj": {"props": ro}}, rules=rules())


def test_a_declarative_converter_can_own_a_writable_property():
    tens = {"props": {"position": {"type": "number", "role": "position", "min": 0, "max": 10, "step": 1, "rw": True}},
            "update": [{"emit": "position", "value": {"op": "floor", "args": [
                {"op": "div", "args": [{"op": "get", "path": "codes.percent_control"}, 10]}]}}],
            "write": {"position": [{"send": {"code": "percent_control",
                                             "value": {"op": "mul", "args": [{"op": "get", "path": "value"}, 10]}}}]}}
    d = TuyaDriver(curtain(), overrides={"cur1": {"converters": {"declarative": tens}}}, rules=rules())
    assert d.descriptor["props"]["position"]["max"] == 10
    d.handle(0, Connected())
    assert _values(d.handle(1, Message("active", {"2": 40})))["position"] == 4
    assert _sent(d.handle(2, Command("position", 7))) == [{"2": 70}]


@needs_spec
def test_a_defined_cover_validates():
    jsonschema = pytest.importorskip("jsonschema")
    jsonschema.validate(TuyaDriver(PRODUCTS["5rta89nj"], overrides=WINDOW, rules=rules()).descriptor, json.loads(SCHEMA.read_text()))
