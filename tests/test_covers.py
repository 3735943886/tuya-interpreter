"""A cover's settings (`cover` block): the device's switches, what each does, motion state on top, and settings changes returned as data for the host to keep."""

import pytest
from helpers import curtain, fn, strat
from rule_data import rules

from tuya2ildevice import (
    Command,
    Connected,
    Disconnected,
    Message,
    OverrideError,
    SendMessage,
    SettingsChanged,
    TuyaDriver,
    Value,
)

PCT = {"unit": "%", "min": 0, "max": 100, "scale": 0, "step": 1}
SWITCH_KEYS = ("cover_invert_position", "cover_invert_set_position", "cover_invert_control", "cover_state_source", "cover_position_from_target")


def cover(dev_id="c1", category="cl", fns=(), status=(), pid="p", dps=None):
    """A cover of `fns` (writable) and `status` (read only) dps: [(code, type, values)]; dp ids in order from 1."""
    f = {c: fn(c, t, **v) for c, t, v in fns}
    sr = {**f, **{c: fn(c, t, **v) for c, t, v in status}}
    ids = dps or {str(i + 1): (c, fn_["type"]) for i, (c, fn_) in enumerate(sr.items())}
    return {"id": dev_id, "category": category, "product_id": pid, "name": "Cover", "product_name": "Cover",
            "function": f, "status_range": sr, "status": {}, "local_strategy": strat(ids)}


CONTROL = ("control", "Enum", {"range": ["open", "stop", "close"]})
ONE_DP = cover(fns=[CONTROL, ("percent_control", "Integer", PCT)])                         # 1 control, 2 position
READ_ONLY = cover(fns=[CONTROL], status=[("percent_state", "Integer", PCT)])                 # 1 control, 2 position
CONTROL_ONLY = cover(fns=[CONTROL])


def switches(desc):
    return {k for k in desc["props"] if "cover_" in k and k.endswith(SWITCH_KEYS)}


def values(outs):
    return {o.prop: o.value for o in outs if isinstance(o, Value)}


def sent(outs):
    return next(o.json["dps"] for o in outs if isinstance(o, SendMessage))


def drive(device, block=None, **kw):
    d = TuyaDriver(device, overrides={device["id"]: {"cover": block}} if block is not None else None,
                   device_settings=True, **kw, rules=rules())
    d.handle(0, Connected())
    return d


# ---- which switches a cover gets ---------------------------------------------------------------------------------
@pytest.mark.parametrize("device, expected", [
    (curtain(), {"cover_invert_position", "cover_invert_set_position", "cover_state_source", "cover_position_from_target"}),   # reported + target
    (ONE_DP, {"cover_invert_position", "cover_state_source"}),                                    # one dp for both
    (READ_ONLY, {"cover_invert_position", "cover_invert_control", "cover_state_source"}),         # position + control
    (CONTROL_ONLY, {"cover_invert_control", "cover_state_source"}),
])
def test_a_switch_is_offered_only_where_it_changes_something(device, expected):
    desc = TuyaDriver(device, device_settings=True, rules=rules()).descriptor
    assert switches(desc) == expected
    for k in expected:
        assert desc["props"][k]["type"] == ("select" if k == "cover_state_source" else "binary") and desc["props"][k]["rw"] and desc["props"][k]["category"] == "config"
    assert desc["props"]["cover_invert_position"]["label"] == "Invert current position" if "cover_invert_position" in expected else True
    assert ("cover_state" in desc["props"]) == (device != CONTROL_ONLY)          # inferred by default


def test_no_switch_without_device_settings_and_read_only_for_a_hazardous_cover():
    props = TuyaDriver(curtain(), rules=rules()).descriptor["props"]
    assert not [p for p in props if p.startswith("cover_")]              # no motion inferred unless a block asks
    assert "cover_state" in TuyaDriver(curtain(), overrides={"p": {"cover": {"infer_motion": True}}}, rules=rules()).descriptor["props"]
    garage = cover(category="ckmkzq", fns=[("switch_1", "Boolean", {})], status=[("doorcontact_state", "Boolean", {})])
    props = TuyaDriver(garage, device_settings=True, rules=rules()).descriptor["props"]
    assert "cover_invert_control" in props and "rw" not in props["cover_invert_control"]
    d = drive(garage)
    assert d.handle(1, Command("cover_invert_control", True))[0].code == "read_only"
    assert TuyaDriver(garage, device_settings=True, allow_hazardous=True, rules=rules()).descriptor["props"]["cover_invert_control"]["rw"]


def test_the_switches_show_the_settings_in_effect_even_offline():
    d = TuyaDriver(curtain(), overrides={"p": {"cover": {"invert_position": True, "infer_motion": False}}},
                   device_settings=True, rules=rules())
    outs = values(d.describe(now=0))
    assert outs["cover_invert_position"] is True and outs["cover_invert_set_position"] is False
    assert outs["available"] is False and "cover_state_source" in d.descriptor["props"]
    d.handle(0, Connected())
    d.handle(1, Message("state", {"3": 30}))
    outs = d.handle(2, Disconnected())
    assert Value("available", False) in outs and not any(getattr(o, "prop", "").startswith("cover_") for o in outs)


# ---- what each does --------------------------------------------------------------------------------------------------
def test_invert_position_reads_the_reported_dp_the_other_way():
    d = drive(curtain(), {"invert_position": True})
    assert values(d.handle(1, Message("state", {"3": 30})))["position"] == 70
    assert sent(d.handle(2, Command("position", 70))) == {"2": 70}              # the target is a dp of its own


def test_with_one_dp_for_both_invert_position_turns_reads_and_writes():
    d = drive(ONE_DP, {"invert_position": True})
    assert values(d.handle(1, Message("state", {"2": 30})))["position"] == 70
    assert sent(d.handle(2, Command("position", 70))) == {"2": 30}
    assert sent(d.handle(3, Command("open", None))) == {"2": 0}


def test_invert_set_position_writes_and_reads_the_target_the_other_way():
    d = drive(curtain(), {"invert_set_position": True})
    assert sent(d.handle(1, Command("position", 70))) == {"2": 30}
    assert sent(d.handle(2, Command("open", None))) == {"2": 0}
    assert values(d.handle(3, Message("state", {"3": 30})))["position"] == 30     # the reported one stays


@pytest.mark.parametrize("device, open_, close, stop", [
    (CONTROL_ONLY, "close", "open", "stop"),
    (cover(fns=[("mach_operate", "Enum", {"range": ["FZ", "ZZ", "STOP"]})]), "ZZ", "FZ", "STOP"),     # the special one
    (cover(fns=[("switch_1", "Boolean", {})]), False, True, None),                                    # a Boolean
])
def test_invert_control_swaps_open_and_close(device, open_, close, stop):
    d = drive(device, {"invert_control": True})
    (dp,) = device["local_strategy"]
    assert sent(d.handle(1, Command("open", None))) == {dp: open_}
    assert sent(d.handle(2, Command("close", None))) == {dp: close}
    if stop is not None:
        assert sent(d.handle(3, Command("stop", None))) == {dp: stop}


def test_invert_control_composes_with_the_users_alias():
    device = cover(fns=[("control", "Enum", {"range": ["on", "off", "pause"]})])     # device_settings off: as a file
    block = {"remap": {"control": {"alias": {"on": "open", "off": "close", "pause": "stop"}}}}
    plain = TuyaDriver(device, overrides={"c1": block}, rules=rules())
    plain.handle(0, Connected())
    assert sent(plain.handle(1, Command("open", None))) == {"1": "on"}
    d = TuyaDriver(device, overrides={"c1": {**block, "cover": {"invert_control": True}}}, rules=rules())
    d.handle(0, Connected())
    assert sent(d.handle(1, Command("open", None))) == {"1": "off"}
    assert sent(d.handle(2, Command("close", None))) == {"1": "on"}
    assert sent(d.handle(3, Command("stop", None))) == {"1": "pause"}


def test_a_setting_xors_with_an_old_remap_invert_returns_warnings():
    block = {"remap": {"percent_state": {"invert": True}}, "cover": {"invert_position": True}}
    d = TuyaDriver(curtain() | {"id": "warn1"}, overrides={"warn1": block}, rules=rules())
    d.handle(0, Connected())
    assert values(d.handle(1, Message("state", {"3": 30})))["position"] == 30      # turned twice
    assert any("remap.percent_state.invert on a cover is deprecated" in warning for warning in d.warnings)
    legacy = TuyaDriver(curtain() | {"id": "warn2"}, overrides={"warn2": {"converters": {"cover_motion": {}}}}, rules=rules())
    assert any("converters.cover_motion is deprecated" in warning for warning in legacy.warnings)
    lamp = cover(dev_id="warn3", category="kg", fns=[("switch_1", "Boolean", {})])
    normal = TuyaDriver(lamp, overrides={"warn3": {"remap": {"switch_1": {"invert": True}}}}, rules=rules())   # not a cover: as it was
    assert normal.warnings == []


def test_position_from_target_and_invert_tilt():
    d = drive(curtain(), {"position_from_target": True})
    assert values(d.handle(1, Message("state", {"2": 40, "3": 10})))["position"] == 40
    assert "cover_invert_set_position" not in d.descriptor["props"]           # one dp for both now
    tilted = cover(fns=[CONTROL, ("percent_control", "Integer", PCT), ("angle_horizontal", "Integer", PCT)])
    d = drive(tilted, {"invert_tilt": True})
    assert values(d.handle(1, Message("state", {"3": 25})))["tilt"] == 75


def test_bad_cover_blocks_are_refused():
    for bad in ({"invert_positon": True}, {"invert_position": "yes"}, {"settle": -1}, {"control_2": {"x": True}},
                {"control_2": {"control_3": {}}}):
        with pytest.raises(OverrideError):
            TuyaDriver(curtain(), overrides={"cur1": {"cover": bad}}, rules=rules())
    with pytest.raises(OverrideError, match="no cover in a group"):
        TuyaDriver(curtain(), overrides={"cur1": {"cover": {"control_2": {"invert_position": True}}}}, rules=rules())


# ---- CoverMotion on top ----------------------------------------------------------------------------------------------
def motion(outs, prop="cover_state"):
    return [o.value for o in outs if isinstance(o, Value) and o.prop == prop]


def test_motion_is_inferred_after_the_inversion():
    d = drive(curtain(), {"invert_position": True, "invert_set_position": True})
    assert motion(d.handle(1, Message("state", {"2": 70, "3": 70}))) == ["stopped"]      # 30%
    assert motion(d.handle(2, Message("active", {"2": 0}))) == ["opening"]               # target 100%
    assert motion(d.handle(3, Message("active", {"3": 0}))) == ["open"]


def test_a_command_word_starts_a_move_and_the_position_decides_its_way():
    d = drive(READ_ONLY)
    d.handle(1, Message("state", {"1": "stop", "2": 50}))
    assert motion(d.handle(2, Message("active", {"1": "open"}))) == ["opening"]          # the device says open...
    assert motion(d.handle(3, Message("active", {"2": 40}))) == ["closing"]              # ...and closes
    assert motion(d.handle(4, Message("active", {"2": 20}))) == []
    assert motion(d.handle(5, Message("active", {"2": 0}))) == ["closed"]
    assert motion(d.handle(6, Message("active", {"1": "close"}))) == []                  # at the end already


def test_infer_motion_off_leaves_no_cover_state_and_settle_is_passed_on():
    assert "cover_state" not in drive(curtain(), {"infer_motion": False}).descriptor["props"]
    d = drive(curtain(), {"settle": 4})
    d.handle(1, Message("state", {"3": 50}))
    outs = d.handle(2, Message("active", {"1": "close"}))
    assert motion(outs) == ["closing"] and any(getattr(o, "after", None) == 4 for o in outs)


def test_a_users_cover_motion_wins_over_the_setting():
    d = TuyaDriver(curtain(), overrides={"cur1": {"converters": {"cover_motion": {"invert": True}}}},
                   device_settings=True, rules=rules())
    assert "cover_state_source" not in d.descriptor["props"]                        # it would change nothing
    d.handle(0, Connected())
    assert motion(d.handle(1, Message("state", {"3": 100}))) == ["closed"]          # the user's own config


# ---- several covers ------------------------------------------------------------------------------------------------
TWO = cover(fns=[CONTROL, ("percent_control", "Integer", PCT), ("control_2", "Enum", {"range": ["open", "stop", "close"]}),
                 ("percent_control_2", "Integer", PCT)],
            status=[("percent_state", "Integer", PCT), ("percent_state_2", "Integer", PCT)])


def test_each_cover_has_its_switches_and_its_block():
    d = TuyaDriver(TWO, overrides={"c1": {"cover": {"control_2": {"invert_position": True}}}}, device_settings=True, rules=rules())
    props = d.descriptor["props"]
    assert props["control_2_cover_invert_position"]["group"] == "control_2" and "cover_invert_position" in props
    assert props["control_2_cover_state"]["group"] == "control_2"
    assert values(d.handle(0, Connected()))["control_2_cover_invert_position"] is True
    v = values(d.handle(1, Message("state", {"5": 30, "6": 30})))
    assert v["position"] == 30 and v["control_2_position"] == 70
    outs = d.handle(2, Command("cover_invert_set_position", True))
    assert SettingsChanged({"cover": {"invert_set_position": True, "control_2": {"invert_position": True}}}) in outs


# ---- writing a switch ------------------------------------------------------------------------------------------------
def test_writing_a_switch_sends_nothing_and_reports_the_block():
    d = drive(curtain())
    assert d.handle(1, Command("cover_invert_position", True)) == [
        Value("cover_invert_position", True), SettingsChanged({"cover": {"invert_position": True}})]
    assert d.handle(2, Command("cover_invert_position", False)) == [
        Value("cover_invert_position", False), SettingsChanged({})]              # back where it started: no block
    d.handle(3, Disconnected())
    assert d.handle(4, Command("cover_state_source", "none"))[-1] == SettingsChanged({"cover": {"state_source": "none"}})


def test_a_switch_keeps_what_differs_from_the_products_block_and_what_the_device_had():
    ov = {"p": {"cover": {"invert_position": True}}, "cur1": {"cover": {"settle": 3}}}
    d = TuyaDriver(curtain(), overrides=ov, device_settings=True, rules=rules())
    assert d.handle(0, Command("cover_invert_position", False))[-1] == \
        SettingsChanged({"cover": {"invert_position": False, "settle": 3}})


@pytest.mark.parametrize("device, words", [
    (CONTROL_ONLY, ["open", "close", "stop"]),
    (cover(fns=[("mach_operate", "Enum", {"range": ["FZ", "ZZ", "STOP"]})]), ["FZ", "ZZ", "STOP"]),
    (cover(fns=[("switch_1", "Boolean", {})]), [True, False]),
])
@pytest.mark.parametrize("inverse", [False, True])
def test_direct_reports_without_position(device, words, inverse):
    d = drive(device, {"state_source": "control", "invert_reported_motion": inverse})
    assert motion(d.handle(1, Message("state", {"1": words[0]}))) == []
    for word, state in zip(words, ["closing", "opening", "stopped"] if inverse else ["opening", "closing", "stopped"]):
        assert motion(d.handle(2, Message("active", {"1": word}))) == [state]
        assert motion(d.handle(3, Message("active", {"1": word}))) == []
    d.handle(4, Disconnected())
    d.handle(5, Connected())
    assert motion(d.handle(6, Message("state", {"1": words[0]}))) == []
    assert motion(d.handle(7, Message("active", {"1": words[0]}))) == ["closing" if inverse else "opening"]
    assert not d.timers


def test_direct_reports_do_not_use_position_or_change_commands():
    normal = drive(curtain(), {"state_source": "control", "position_from_target": True, "invert_position": True})
    reverse = drive(curtain(), {"state_source": "control", "position_from_target": True, "invert_position": True,
                               "invert_reported_motion": True})
    for d in (normal, reverse):
        assert values(d.handle(1, Message("state", {"2": 100, "3": 50})))["position"] == 0
        assert motion(d.handle(2, Message("active", {"1": "open"}))) == ["closing" if d is reverse else "opening"]
        assert motion(d.handle(3, Message("active", {"2": 0}))) == []
        assert motion(d.handle(4, Message("active", {"1": "stop"}))) == ["stopped"]
    for prop, value, expected in [("open", None, {"2": 0}), ("close", None, {"2": 100}),
                                  ("stop", None, {"1": "stop"}), ("position", 70, {"2": 30})]:
        assert sent(normal.handle(5, Command(prop, value))) == sent(reverse.handle(5, Command(prop, value))) == expected


def test_alias_then_control_inversion_then_report_inversion():
    device = cover(fns=[("control", "Enum", {"range": ["on", "off", "pause"]})])
    for control_inv in (False, True):
        for report_inv in (False, True):
            d = TuyaDriver(device, device_settings=True, overrides={"c1": {
                "remap": {"control": {"alias": {"on": "open", "off": "close", "pause": "stop"}}},
                "cover": {"state_source": "control", "invert_control": control_inv, "invert_reported_motion": report_inv}}}, rules=rules())
            d.handle(0, Connected())
            assert motion(d.handle(1, Message("active", {"1": "on"}))) == ["closing" if control_inv != report_inv else "opening"]
            assert sent(d.handle(2, Command("open", None))) == {"1": "off" if control_inv else "on"}
            assert motion(d.handle(3, Message("active", {"1": "pause"}))) == ["stopped"]


def test_none_only_derives_rest_and_legacy_setting_maps_to_select():
    for enabled in (True, False):
        d = drive(curtain(), {"infer_motion": enabled})
        settings = d.state["covers"][0]["settings"]
        assert settings["state_source"] is None and settings["infer_motion"] is enabled
        assert "cover_infer_motion" not in d.descriptor["props"]
    d = drive(curtain(), {"state_source": "none", "infer_motion": True})
    assert motion(d.handle(1, Message("active", {"1": "open", "3": 0}))) == ["closed"]
    assert motion(d.handle(2, Message("active", {"1": "close", "3": 100}))) == ["open"]
    assert motion(d.handle(3, Message("active", {"3": 50}))) == ["stopped"]
    assert not d.timers


def test_user_converter_owns_only_its_cover():
    d = TuyaDriver(TWO, device_settings=True, overrides={"c1": {
        "cover": {"state_source": "control", "control_2": {"state_source": "control", "invert_reported_motion": True}},
        "converters": {"cover_motion": {}}}}, rules=rules())
    d.handle(0, Connected())
    assert "cover_state_source" not in d.descriptor["props"]
    assert "control_2_cover_state_source" in d.descriptor["props"]
    out = d.handle(1, Message("active", {"3": "open"}))
    assert motion(out, "control_2_cover_state") == ["closing"]
    assert motion(out) == []


def test_multiple_cover_position_choices_and_saved_settings_are_independent():
    d = drive(TWO, {"invert_position": True, "control_2": {
        "position_from_target": True, "invert_position": True, "state_source": "control"}})
    assert [c["cur"]["code"] for c in d.state["covers"]] == ["percent_state", "percent_control_2"]
    assert "cover_position_from_target" in d.descriptor["props"]
    assert "control_2_cover_position_from_target" in d.descriptor["props"]
    assert values(d.handle(1, Message("state", {"2": 20, "4": 30, "5": 40, "6": 50})))["position"] == 60
    assert d.state["values"]["control_2_position"] == 70
    outs = d.handle(2, Command("control_2_cover_invert_reported_motion", True))
    saved = next(o.block for o in outs if isinstance(o, SettingsChanged))
    assert saved["cover"]["invert_position"] is True
    assert saved["cover"]["control_2"]["invert_reported_motion"] is True
    assert "invert_reported_motion" not in saved["cover"]


def test_shared_position_is_never_compared_to_itself_for_motion():
    d = drive(ONE_DP, {"state_source": "inferred"})
    assert motion(d.handle(1, Message("active", {"2": 40}))) == ["stopped"]
    assert motion(d.handle(2, Message("active", {"2": 80}))) == []
    assert not d.timers


@pytest.mark.parametrize("bad", ["invalid", True, None, {}, []])
def test_invalid_state_source_is_rejected(bad):
    with pytest.raises(OverrideError):
        drive(curtain(), {"state_source": bad})
