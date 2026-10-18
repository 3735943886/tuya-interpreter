import json
import os
import pathlib

import pytest
from helpers import curtain, fn, light, strat
from rule_data import rules

from tuya_interpreter import (
    Absent,
    Command,
    Connected,
    Disconnected,
    Event,
    Message,
    Reject,
    Rejected,
    SendMessage,
    TuyaDriver,
    Value,
    check_command,
)

ILDEVICE = pathlib.Path(os.environ.get("ILDEVICE", pathlib.Path(__file__).resolve().parents[2] / "ildevice"))   # the spec repo
# An explicit ILDEVICE (as in CI) must exist: a missing checkout fails rather than skips.
needs_spec = pytest.mark.skipif("ILDEVICE" not in os.environ and not ILDEVICE.is_dir(),
                                reason=f"no ildevice checkout at {ILDEVICE} (set ILDEVICE)")


def test_light_realtime_and_commands():
    d = TuyaDriver(light(), rules=rules())
    props = d.descriptor["props"]
    assert d.descriptor["kind"] == "light"
    assert props["switch_led"]["role"] == "on" and props["brightness"]["max"] == 100
    outs = d.handle(0, Connected())
    assert outs[0].desc == d.descriptor
    outs = d.handle(1, Message("state", {"20": True, "22": 1000, "23": 0}))
    vals = {o.prop: o.value for o in outs if isinstance(o, Value)}
    assert vals["available"] is True and vals["switch_led"] is True and vals["brightness"] == 100
    # active delta; only the changed prop is emitted
    assert d.handle(2, Message("active", {"22": 505})) == [Value("brightness", 50)]
    assert d.handle(3, Message("passive", {"22": 505})) == []          # no change, no repeat
    (out,) = d.handle(4, Command("brightness", 50))
    assert isinstance(out, SendMessage) and out.json["dps"]["22"] == 507 and out.json["dps"]["20"] is True   # 50% -> 128/255 -> 507
    assert d.handle(5, Command("brightness", 101))[0].code == "out_of_range"
    assert d.handle(5, Command("switch_led", "maybe"))[0].code == "invalid_value"


def test_cover_position_and_triggers():
    d = TuyaDriver(curtain(), rules=rules())
    p = d.descriptor["props"]
    assert d.descriptor["kind"] == "cover" and p["position"]["rw"] and {"open", "close", "stop"} <= set(p)
    d.handle(0, Connected())
    # unlike core, the position is the device's own number (no mirroring): raw 30 is 30
    outs = d.handle(1, Message("active", {"3": 30}))
    assert Value("position", 30) in outs
    (out,) = d.handle(2, Command("open"  , None))
    assert out.json == {"dps": {"2": 100}}         # set_position wins over the instruction dp, as in core
    assert d.handle(3, Command("position", 70))[0].json == {"dps": {"2": 70}}
    inv = TuyaDriver(curtain(), overrides={"cur1": {"remap": {"percent_state": {"invert": True},
                                                              "percent_control": {"invert": True}}}}, rules=rules())
    inv.handle(0, Connected())
    assert Value("position", 70) in inv.handle(1, Message("active", {"3": 30}))      # the other way: asked for
    assert inv.handle(2, Command("position", 70))[0].json == {"dps": {"2": 30}}


def test_unavailable_when_disconnected_and_absent_on_drop():
    d = TuyaDriver(curtain(), rules=rules())
    assert d.handle(0, Command("position", 10))[0] == Reject("position", "unavailable", "device link is down")
    d.handle(0, Connected())
    d.handle(1, Message("state", {"3": 30}))
    outs = d.handle(2, Disconnected())
    assert Absent("position") in outs and Value("available", False) in outs


def test_command_checks_before_link():
    d = TuyaDriver(curtain(), rules=rules())
    assert d.handle(0, Command("nope", 1))[0].code == "unknown_property"
    assert d.handle(0, Command("available", True))[0].code == "read_only"


def test_delta_and_event_only_on_active():
    f = {"add_ele": {"code": "add_ele", "type": "Integer", "values": json.dumps({"min": 0, "max": 50000, "scale": 3, "step": 1}), "report_type": "sum"}}
    dev = {"id": "m", "category": "cz", "product_id": "p", "name": "Plug", "function": {}, "status_range": f, "status": {},
           "local_strategy": strat({"17": ("add_ele", "Integer")})}
    d = TuyaDriver(dev, rules=rules())
    assert d.descriptor["props"]["add_ele"]["series"] == "counter"
    d.handle(0, Connected())
    assert Value("add_ele", 0) in d.handle(1, Message("state", {"17": 84}))          # readback never accumulates
    assert d.handle(2, Message("active", {"17": 5, "t": 1})) == [Value("add_ele", 0.005)]
    assert d.handle(3, Message("active", {"17": 7, "t": 2})) == [Value("add_ele", 0.012)]


def test_event_fires_on_active_only():
    f = {"switch_mode1": fn("switch_mode1", "Enum", range=["click", "double_click", "press"])}
    dev = {"id": "b", "category": "wxkg", "product_id": "p", "name": "Btn", "function": {}, "status_range": f, "status": {},
           "local_strategy": strat({"1": ("switch_mode1", "Enum")})}
    d = TuyaDriver(dev, rules=rules())
    assert d.descriptor["props"]["switch_mode1"] == {"type": "event", "options": ["click", "double_click", "press"],
                                                     "src": "switch_mode1", "class": "button"}
    d.handle(0, Connected())
    assert d.handle(1, Message("state", {"1": "click"})) == [Value("available", True)]      # a replay is no occurrence
    assert d.handle(2, Message("active", {"1": "click"})) == [Event("switch_mode1", "click")]
    assert d.handle(3, Message("active", {"1": "click"})) == [Event("switch_mode1", "click")]   # same kind twice: two events
    assert d.handle(4, Message("passive", {"1": "double_click"})) == []      # a passive report is no press, even changed


# --- il.md section 5 vectors ------------------------------------------------------------------
VECTORS = ILDEVICE / "vectors" / "commands.json"


@needs_spec
def test_il_command_vectors():
    v = json.loads(VECTORS.read_text())
    for c in v["cases"]:
        try:
            got = {"accept": check_command(v["descriptor"], c.get("state", {}), c["prop"], c.get("value"))}
        except Rejected as e:
            got = {"reject": e.code}
        assert got == c["expect"], c["name"]


# --- descriptors of the real devices validate against the IL schema ------------------------------
SCHEMA = ILDEVICE / "schema" / "descriptor.schema.json"


@needs_spec
def test_every_core_fixture_yields_a_valid_descriptor():
    """All 324 Home Assistant core tuya fixtures: descriptor validates, and carries no secret (D-3)."""
    import fixtures
    jsonschema = pytest.importorskip("jsonschema")
    schema = json.loads(SCHEMA.read_text())
    for code in fixtures.all_codes():
        d = TuyaDriver(fixtures.load(code), rules=rules())
        jsonschema.validate(d.descriptor, schema)
        assert "local_key" not in json.dumps(d.descriptor)


def test_a_malformed_colour_is_rejected_not_raised():
    """A `color` write that is not `#rrggbb` is an `invalid_value` Reject, as any value the engine refuses."""
    import fixtures
    seen = 0
    for code in fixtures.all_codes():
        d = TuyaDriver(fixtures.load(code), rules=rules())
        if "color" not in d.descriptor["props"]:
            continue
        seen += 1
        d.handle(0, Connected())
        for bad in ("x", "#12", "#gggggg"):
            (out,) = d.handle(1, Command("color", bad))
            assert isinstance(out, Reject) and out.code == "invalid_value", (code, bad, out)
    assert seen


def test_unused_dps_get_properties_by_type():
    f = {"switch_1": fn("switch_1", "Boolean"), "countdown_1": fn("countdown_1", "Integer", unit="s", min=0, max=86400, scale=0, step=1),
         "mystery": fn("mystery", "Enum", range=["a", "b"]), "cycle_time": fn("cycle_time", "String")}
    sr = {**f, "fault": fn("fault", "Bitmap", label=["e1", "e2"]), "temp_x": fn("temp_x", "Integer", unit="", min=0, max=100, scale=1, step=1)}
    dev = {"id": "x", "category": "zz", "product_id": "p", "name": "X", "function": f, "status_range": sr, "status": {},
           "local_strategy": strat({"1": ("switch_1", "Boolean"), "7": ("countdown_1", "Integer"), "9": ("mystery", "Enum"),
                                    "10": ("cycle_time", "String"), "11": ("fault", "Bitmap"), "12": ("temp_x", "Integer")})}
    assert "mystery" not in TuyaDriver(dev, rules=rules()).descriptor["props"]           # off by default: core creates no entity for it
    d = TuyaDriver(dev, expose_unused=True, rules=rules())
    p = d.descriptor["props"]
    assert p["mystery"] == {"type": "select", "rw": True, "options": ["a", "b"], "label": "Mystery", "category": "config", "src": "mystery"}
    assert p["countdown_1"]["step"] == 1 and isinstance(p["countdown_1"]["step"], int) and p["countdown_1"]["unit"] == "s"
    assert "rw" not in p["cycle_time"] and p["cycle_time"]["type"] == "text" and p["cycle_time"]["category"] == "diagnostic"
    assert p["fault"]["type"] == "number" and p["temp_x"]["category"] == "diagnostic" and "rw" not in p["temp_x"]
    d.handle(0, Connected())
    outs = d.handle(1, Message("state", {"9": "b", "12": 55, "11": 2, "10": "abc"}))
    vals = {o.prop: o.value for o in outs if isinstance(o, Value)}
    assert vals["mystery"] == "b" and vals["temp_x"] == 5.5 and vals["fault"] == 2 and vals["cycle_time"] == "abc"
    assert d.handle(2, Command("mystery", "a"))[0].json == {"dps": {"9": "a"}}
    assert d.handle(3, Command("temp_x", 1))[0].code == "read_only"
    

# --- user overrides ----------------------------------------------------------------------------
from tuya_interpreter import OverrideError, merge_all


def _kg():
    f = {"switch_1": fn("switch_1", "Boolean"), "countdown_1": fn("countdown_1", "Integer", unit="s", min=0, max=86400, scale=0, step=1),
         "cycle_time": fn("cycle_time", "String")}
    return {"id": "d1", "category": "kg", "product_id": "prod1", "name": "Sw", "function": f, "status_range": f, "status": {},
            "local_strategy": strat({"1": ("switch_1", "Boolean"), "7": ("countdown_1", "Integer"), "10": ("cycle_time", "String")})}


def test_override_props_device_and_precedence():
    ovr = {"prod1": {"props": {"countdown_1": {"label": "Timer", "category": None, "unit": "min"}, "cycle_time": {"hide": True}},
                     "device": {"model": "X1"}},
           "d1": {"props": {"switch_1": {"rw": False, "class": "switch"}}}}
    d = TuyaDriver(_kg(), overrides=ovr, expose_unused=True, rules=rules())
    p = d.descriptor["props"]
    assert p["countdown_1"]["label"] == "Timer" and p["countdown_1"]["unit"] == "min" and "category" not in p["countdown_1"]
    assert "cycle_time" not in p and d.descriptor["model"] == "X1"
    assert "rw" not in p["switch_1"] and p["switch_1"]["class"] == "switch"
    d.handle(0, Connected())
    assert d.handle(1, Command("switch_1", True))[0].code == "read_only"
    outs = d.handle(2, Message("state", {"10": "x"}))
    assert all(getattr(o, "prop", "") != "cycle_time" for o in outs)              # hidden: no value either


def test_override_dp_definition_is_classified_and_remove_hides_fallback():
    dev = _kg()
    ovr = {"prod1": {"dp": {"104": {"code": "percent_control", "type": "Integer",
                                    "values": {"min": 0, "max": 100, "scale": 0, "step": 1, "unit": "%"}}},
                     "remove": ["cycle_time"]}}
    d = TuyaDriver(dev, overrides=ovr, expose_unused=True, rules=rules())
    assert "cycle_time" not in d.descriptor["props"] and "percent_control" in d.descriptor["props"]
    d.handle(0, Connected())
    outs = d.handle(1, Message("state", {"104": 40}))
    assert Value("percent_control", 40) in outs
    assert d.handle(2, Command("percent_control", 60))[0].json == {"dps": {"104": 60}}
    # redefining an existing dp id points it at the new code
    d2 = TuyaDriver(dev, expose_unused=True, overrides={"d1": {"dp": {"7": {"code": "renamed", "type": "Integer", "values": {"min": 0, "max": 9, "scale": 0, "step": 1}}}}}, rules=rules())
    assert "renamed" in d2.descriptor["props"] and "countdown_1" not in d2.descriptor["props"]


def test_override_errors_are_loud():
    for bad in ({"prod1": {"prop": {}}}, {"prod1": {"props": {"nope": {"label": "x"}}}},
                {"prod1": {"props": {"switch_1": {"unit": "s"}}}}, {"prod1": {"props": {"switch_1": {"rw": True}}}},
                {"prod1": {"dp": {"9": {"code": "a", "type": "Nonsense"}}}}):
        with pytest.raises(OverrideError):
            TuyaDriver(_kg(), overrides=bad, rules=rules())


def test_merge():
    assert merge_all([{"a": {"props": {"x": {"label": "1", "class": "c"}}}}, {"a": {"props": {"x": {"label": "2"}}}}]) == \
        {"a": {"props": {"x": {"label": "2", "class": "c"}}}}


# --- converters ----------------------------------------------------------------------------------
from tuya_interpreter.io import CancelTimer, SetTimer, Timer


def _motion(outs):
    return [o.value for o in outs if isinstance(o, Value) and o.prop == "cover_state"]


def test_inferred_cover_motion():
    cfg = {"cur1": {"cover": {"state_source": "inferred", "settle": 5}}}
    d = TuyaDriver(curtain(), overrides=cfg, rules=rules())
    assert d.descriptor["props"]["cover_state"] == {"type": "select", "role": "cover_state",
                                                    "options": ["open", "closed", "opening", "closing", "stopped"]}
    d.handle(0, Connected())
    assert _motion(d.handle(1, Message("state", {"3": 0}))) == ["closed"]          # a snapshot never starts motion
    outs = d.handle(2, Message("active", {"1": "open"}))
    assert _motion(outs) == ["opening"] and SetTimer("c0:settle", 5.0) in outs
    assert _motion(d.handle(3, Message("active", {"3": 50}))) == []                # still moving
    outs = d.handle(4, Message("active", {"3": 100}))
    assert _motion(outs) == ["open"] and CancelTimer("c0:settle") in outs          # arrived at the end
    assert _motion(d.handle(5, Message("active", {"2": 20}))) == ["closing"]       # set-position below the current one
    assert _motion(d.handle(6, Message("active", {"3": 20}))) == ["stopped"]       # reached the target
    d.handle(7, Message("active", {"1": "close"}))
    assert _motion(d.handle(8, Timer("c0:settle"))) == ["stopped"]                 # no position report: settle timer
    assert _motion(d.handle(9, Message("state", {"3": 0}))) == ["closed"]          # at rest at an end: that end
    d.handle(9, Message("active", {"1": "open"}))
    outs = d.handle(10, Disconnected())
    assert Absent("cover_state") in outs and CancelTimer("c0:settle") in outs


def test_inferred_motion_follows_engine_position():
    d = TuyaDriver(curtain(), overrides={"cur1": {"cover": {"state_source": "inferred"}}}, rules=rules())
    d.handle(0, Connected())
    assert _motion(d.handle(1, Message("state", {"3": 100}))) == ["open"]          # raw 100 = open, as the position
    assert _motion(d.handle(2, Message("active", {"2": 0}))) == ["closing"]        # raw target 0 = closed


def test_a_live_passive_report_that_changes_a_value_is_a_push():
    d = TuyaDriver(curtain(), overrides={"cur1": {"cover": {"state_source": "inferred"}}}, rules=rules())
    d.handle(0, Connected())
    d.handle(1, Message("state", {"1": "stop", "2": 50, "3": 50}))
    assert _motion(d.handle(2, Message("passive", {"2": 50, "3": 50}))) == []      # a readback: nothing moved
    assert _motion(d.handle(3, Message("passive", {"2": 0, "3": 50}))) == ["closing"]
    assert _motion(d.handle(4, Message("passive", {"3": 20}))) == []               # still moving
    assert _motion(d.handle(5, Message("passive", {"3": 0}))) == ["closed"]


def test_the_bridges_merged_state_copy_does_not_settle_a_move():
    """rustuya-bridge publishes each report on `passive` (or `active`) and again on `state`, its merged view: the copy
    right behind it changes nothing, so the move goes on until the position arrives (a trace from a real curtain)."""
    d = TuyaDriver(curtain(), overrides={"cur1": {"cover": {"state_source": "inferred"}}}, rules=rules())
    d.handle(0, Connected())
    d.handle(1, Message("state", {"1": "close", "2": 10, "3": 10}))
    got = [_motion(d.handle(i + 2, Message(ch, m))) for i, (ch, m) in enumerate([
        ("passive", {"2": 0}), ("state", {"2": 0}), ("passive", {"3": 5}), ("state", {"3": 5}),
        ("passive", {"3": 0}), ("state", {"3": 0})])]
    assert got == [["closing"], [], [], [], ["closed"], []]
    assert _motion(d.handle(9, Message("state", {"2": 60, "3": 60}))) == ["stopped"]     # news only state brought
    d.handle(10, Disconnected())
    d.handle(11, Connected())
    assert _motion(d.handle(12, Message("state", {"1": "close", "2": 60, "3": 60}))) == ["stopped"]  # a fresh snapshot


def test_a_declarative_converter_counts_ticks_and_replaces_a_property():
    count = {"op": "get", "path": "state.n"}
    conv = {"props": {"updates": {"type": "number"}, "position": {"type": "number", "role": "position"}},
            "initial_state": {"n": 0},
            "update": [{"set": "state.n", "value": {"op": "add", "args": [count, 1]}},
                       {"emit": "updates", "value": count}, {"timer": "tick", "after": 10}],
            "timer": [{"emit": "updates", "value": 100}]}
    d = TuyaDriver(curtain(), overrides={"cur1": {"converters": {"declarative": conv}}}, rules=rules())
    assert d.descriptor["props"]["position"] == {"type": "number", "role": "position"}       # replaces the table's
    d.handle(0, Connected())
    outs = d.handle(1, Message("state", {"3": 10}))
    assert Value("updates", 1) in outs and SetTimer("c0:tick", 10.0) in outs
    assert d.handle(2, Timer("c0:tick")) == [Value("updates", 100)]
    assert d.handle(3, Timer("c9:x")) == [] and d.handle(3, Timer("junk")) == []
    with pytest.raises(OverrideError):
        TuyaDriver(curtain(), overrides={"cur1": {"converters": {"nope": {}}}}, rules=rules())
    with pytest.raises(OverrideError):                                  # writable needs a write program
        TuyaDriver(curtain(), overrides={"cur1": {"converters": {"declarative": {
            "props": {"x": {"type": "number", "rw": True}}}}}}, rules=rules())
    hidden = TuyaDriver(curtain(), overrides={"cur1": {"converters": {"declarative": conv},
                                                       "props": {"updates": {"hide": True}}}}, rules=rules())
    hidden.handle(0, Connected())
    assert Value("updates", 1) not in hidden.handle(1, Message("state", {"3": 10}))


# --- alarm panel and robot vacuum ----------------------------------------------------------------
def _with_dpmap(code, **kw):
    """A core fixture has no local_strategy; number its dps by status order."""
    import fixtures
    dev = fixtures.load(code)
    dpmap = {str(i + 1): c for i, c in enumerate(dev["status"])}
    d = TuyaDriver(dev, dpmap=dpmap, **kw, rules=rules())
    d.handle(0, Connected())
    return d, {c: i for i, c in dpmap.items()}


def _sent(d, now, prop, value=None):
    (out,) = d.handle(now, Command(prop, value))
    return out.json["dps"]


def test_alarm_panel_states_and_arming():
    d, ids = _with_dpmap("mal_gyitctrjj1kefxp2", allow_hazardous=True)
    assert d.descriptor["kind"] == "alarm" and d.descriptor["props"]["alarm_state"]["role"] == "alarm_state"
    m = ids["master_mode"]
    assert _sent(d, 1, "arm_home") == {m: "home"} and _sent(d, 2, "arm_away") == {m: "arm"}
    assert _sent(d, 3, "disarm") == {m: "disarmed"}
    def vals(outs):
        return {o.prop: o.value for o in outs if isinstance(o, Value)}
    assert vals(d.handle(4, Message("state", {m: "home"})))["alarm_state"] == "armed_home"
    assert vals(d.handle(5, Message("active", {ids["master_state"]: "alarm", ids["alarm_msg"]: ""})))["alarm_state"] == "triggered"
    import fixtures
    assert "disarm" not in TuyaDriver(fixtures.load("mal_gyitctrjj1kefxp2"), rules=rules()).descriptor["props"]         # S-1


def test_robot_vacuum_state_and_commands():
    d, ids = _with_dpmap("sd_lr33znaodtyarrrz")
    p = d.descriptor["props"]
    assert d.descriptor["kind"] == "vacuum" and p["vacuum_state"]["role"] == "vacuum_state"
    assert [n for n in ("start", "pause", "return_home", "locate") if p[n]["role"] == n] == ["start", "pause", "return_home", "locate"]
    assert _sent(d, 1, "start") == {ids["power_go"]: True} and _sent(d, 2, "stop") == {ids["power_go"]: False}
    assert _sent(d, 3, "pause") == {ids["pause"]: True} and _sent(d, 4, "return_home") == {ids["switch_charge"]: True}
    assert _sent(d, 5, "locate") == {ids["seek"]: True} and _sent(d, 6, "fan_speed", "gentle") == {ids["suction"]: "gentle"}
    vals = {o.prop: o.value for o in d.handle(7, Message("state", {ids["status"]: "cleaning", ids["suction"]: "strong"}))
            if isinstance(o, Value)}
    assert vals["vacuum_state"] == "cleaning" and vals["fan_speed"] == "strong"


def test_after_preload_driving_a_device_reads_no_file(monkeypatch):
    """A host with an event loop that must not block (Home Assistant) imports and preloads in a worker thread; after
    that, making drivers (quirks, platform tables, units) must not touch the disk."""
    import pathlib

    import fixtures

    import tuya_interpreter

    records = [fixtures.load(code) for code in fixtures.all_codes()]      # the fixtures themselves are files
    tuya_interpreter.preload()

    def no_io(*args, **kwargs):
        raise AssertionError(f"file read after preload: {args[:1]}")
    monkeypatch.setattr(pathlib.Path, "read_text", no_io)
    monkeypatch.setattr(pathlib.Path, "open", no_io)
    for rec in records:
        TuyaDriver(rec, allow_hazardous=True, rules=rules())


def test_boolean_values_are_canonical_booleans():
    # Python equality accepts 0 and 1 for a Boolean DP; the published value is a real boolean.
    d = TuyaDriver(light(), rules=rules())
    d.handle(0, Connected())
    assert Value("switch_led", False) in d.handle(1, Message("active", {"20": 0}))
    assert d.handle(2, Message("active", {"20": False})) == []
    assert d.handle(3, Message("active", {"20": 1})) == [Value("switch_led", True)]
    assert d.handle(4, Message("active", {"20": "on"})) == [Absent("switch_led")]


@pytest.mark.parametrize("table", ["function", "status_range", "status", "local_strategy"])
def test_malformed_device_tables_are_rejected(table):
    with pytest.raises(ValueError, match=f"device {table} must be an object"):
        TuyaDriver({**light(), table: []}, rules=rules())
    TuyaDriver({**light(), table: None}, rules=rules())     # absent is allowed


def test_same_config_compares_configuration_not_runtime_state():
    a = TuyaDriver(curtain(), rules=rules())
    b = TuyaDriver(curtain(), rules=rules())
    a.handle(0, Connected())
    a.handle(1, Message("state", {"2": 40, "3": 40}))
    assert a.same_config(b) and b.same_config(a) and a.same_config(a)      # values, link and timers are runtime state
    inverted = TuyaDriver(curtain(), overrides={"p": {"cover": {"invert_position": True}}}, rules=rules())
    assert not a.same_config(inverted)                                    # a cover setting, same descriptor
    labelled = TuyaDriver(curtain(), overrides={"p": {"device": {"label": "Bedroom"}}}, rules=rules())
    assert not a.same_config(labelled)
    other_rules = rules()
    other_rules["overrides"] = {**other_rules.get("overrides", {}), "unused-product": {"device": {"model": "x"}}}
    assert not a.same_config(TuyaDriver(curtain(), rules=other_rules))   # a different rule pack
