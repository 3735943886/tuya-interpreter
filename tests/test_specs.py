"""Device knowledge is data: each change below is a rule layer (what a user file under TUYA_ENGINE_RULES holds) and
takes effect with the same engine library, no rebuild."""
import fixtures

from tuya2ildevice import Command, Connected, Message, SendMessage, TuyaDriver, Value
from tuya2ildevice.native import _call, rules


def layered(*layers):
    return _call({"op": "merge_rules", "base": rules(), "layers": list(layers)})["rules"]


def get(path):
    return {"op": "get", "path": path}


def values(outs):
    return {o.prop: o.value for o in outs if isinstance(o, Value)}


def numbered(dev):
    """A core fixture has no local_strategy; number its dps by status order."""
    return {str(i + 1): c for i, c in enumerate(dev["status"])}


def test_a_category_joins_a_platform_table():
    dev = {"id": "d", "category": "zz", "function": {"switch_1": {"type": "Boolean", "values": "{}"}}}
    assert "switch_1" not in TuyaDriver(dev).descriptor["props"]
    pack = layered({"platforms": {"switch": {"categories": {"zz": [{"key": "switch_1"}]}}}})
    assert TuyaDriver(dev, rules=pack).descriptor["props"]["switch_1"]["type"] == "binary"


def test_a_role_takes_another_dp_code():
    rng = '{"unit":"℃","min":0,"max":500,"scale":1,"step":5}'
    dev = {"id": "d", "category": "kt", "function": {"temp_set": {"type": "Integer", "values": rng}},
           "status_range": {"room_temperature": {"type": "Integer", "values": rng}}}
    assert "current_temperature" not in TuyaDriver(dev).descriptor["props"]
    pack = layered({"entities": {"climate": {"params": {"codes": {"cur_temp_c": {"$append": ["room_temperature"]}}}}}})
    d = TuyaDriver(dev, rules=pack, dpmap={"1": "temp_set", "2": "room_temperature"})
    assert d.descriptor["props"]["current_temperature"]["unit"] == "°C"
    d.handle(0, Connected())
    assert values(d.handle(1, Message("state", {"2": 215})))["current_temperature"] == 21.5


def test_a_platform_gains_a_role_state_action_and_property():
    dev = fixtures.load("sd_lr33znaodtyarrrz")
    ids = {c: i for i, c in numbered(dev).items()}
    pack = layered({"entities": {"vacuum": {
        "params": {"codes": {"resume": ["break_clean"]}},
        "finish": {"$append": [{"role": "resume", "codes": get("entity.params.codes.resume"), "types": "Boolean",
                                "prefer": "function"}]},
        "state": {"fields": {"resume": get("values.resume")}},
        "actions": {"set_resume": [{"command": {"role": "resume", "value": get("args.on")}}]},
        "layout": {"$append": [{
            "name": "resume", "when": {"op": "contains", "args": [get("roles"), "resume"]},
            "definition": {"type": "binary", "rw": True},
            "read": get("state.resume"),
            "write": {"op": "record", "fields": {"action": "set_resume",
                                                 "args": {"op": "record", "fields": {"on": get("value")}}}}}]}}}})
    d = TuyaDriver(dev, rules=pack, dpmap=numbered(dev))
    assert {k: d.descriptor["props"]["resume"][k] for k in ("type", "rw")} == {"type": "binary", "rw": True}
    d.handle(0, Connected())
    assert values(d.handle(1, Message("state", {ids["break_clean"]: True})))["resume"] is True
    assert d.handle(2, Command("resume", False)) == [SendMessage("set", {"dps": {ids["break_clean"]: False}})]


def test_a_state_word_is_mapped():
    dev = fixtures.load("sd_lr33znaodtyarrrz")
    ids = {c: i for i, c in numbered(dev).items()}
    d = TuyaDriver(dev, dpmap=numbered(dev))
    d.handle(0, Connected())
    assert values(d.handle(1, Message("state", {ids["status"]: "goto_pos"})))["vacuum_state"] == "cleaning"
    d = TuyaDriver(dev, rules=layered({"maps": {"_VAC": {"goto_pos": "returning"}}}), dpmap=numbered(dev))
    d.handle(0, Connected())
    assert values(d.handle(1, Message("state", {ids["status"]: "goto_pos"})))["vacuum_state"] == "returning"


def test_a_whole_platform_is_defined_in_json():
    mower = {
        "params": {"codes": {"run": ["power_go"], "status": ["work_state"]}},
        "composite": True, "kind": "lawn_mower", "update_all": True,
        "variants": [{"steps": [
            {"role": "run", "codes": get("entity.params.codes.run"), "types": "Boolean", "prefer": "function",
             "required": True},
            {"role": "status", "codes": get("entity.params.codes.status"), "types": "Enum", "prefer": "status_range"}]}],
        "state": {"fields": {"activity": {"op": "lookup", "args": [{"mowing": "mowing", "charging": "docked"},
                                                                   get("values.status")]}}},
        "actions": {"start": [{"command": {"role": "run", "value": True}}]},
        "layout": [
            {"name": "activity", "definition": {"type": "select", "options": ["mowing", "docked"]},
             "read": get("state.activity")},
            {"name": "start", "definition": {"type": "trigger"}, "write": {"op": "record", "fields": {"action": "start"}}}],
    }
    pack = layered({"platforms": {"lawn_mower": {"table": "MOWERS", "categories": {"gcj": {"key": ""}}}},
                    "entities": {"lawn_mower": mower}})
    dev = {"id": "m", "category": "gcj", "function": {"power_go": {"type": "Boolean", "values": "{}"}},
           "status_range": {"work_state": {"type": "Enum", "values": '{"range":["mowing","charging"]}'}}}
    d = TuyaDriver(dev, rules=pack, dpmap={"1": "power_go", "2": "work_state"})
    assert d.descriptor["kind"] == "lawn_mower" and set(d.descriptor["props"]) == {"available", "activity", "start"}
    d.handle(0, Connected())
    assert values(d.handle(1, Message("state", {"2": "charging"})))["activity"] == "docked"
    assert d.handle(2, Command("start")) == [SendMessage("set", {"dps": {"1": True}})]
