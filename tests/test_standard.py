"""Tuya's own category list applied over core's tables when the rule pack is built (scripts/standard.py)."""
import json

import standard
from engine import table


def test_the_category_list_is_tuyas_own_and_complete():
    cats = standard.categories()
    assert len(cats) == 103   # the page's table, header excluded
    assert (cats["kg"], cats["cz"], cats["pc"]) == ("Switch", "Socket", "Power strip")


def test_every_name_a_rule_uses_is_a_name_in_the_list():
    by_name = standard._rules()["switch_device_class"]["by_name"]
    assert set(by_name) <= set(standard.categories().values())


def test_a_wall_switch_is_a_switch_and_a_socket_or_power_strip_an_outlet():
    assert standard.switch_device_class("kg", "outlet") == "switch"
    assert standard.switch_device_class("cz", "outlet") == "outlet"
    assert standard.switch_device_class("pc", "outlet") == "outlet"


def test_what_the_standard_does_not_name_stays_as_core_has_it():
    assert "tdq" not in standard.categories()
    assert standard.switch_device_class("tdq", "outlet") == "outlet"      # category the page does not list
    assert standard.switch_device_class("kg", None) is None               # a description without a class
    assert standard.switch_device_class("kg", "switch") == "switch"       # only a socket-like class is replaced


def test_apply_changes_only_a_switch_tables_socket_like_classes():
    tables = {"SWITCHES": {"kg": [{"key": "switch_1", "device_class": "outlet"}, {"key": "child_lock"}],
                           "cz": [{"key": "switch", "device_class": "outlet"}]}}
    out = standard.apply("switch", json.loads(json.dumps(tables)))["SWITCHES"]
    assert out["kg"] == [{"key": "switch_1", "device_class": "switch"}, {"key": "child_lock"}]
    assert out["cz"] == tables["SWITCHES"]["cz"]
    other = json.loads(json.dumps(tables))
    assert standard.apply("select", other) == tables


def test_the_rule_pack_carries_it():
    kg = table("switch")["kg"]
    assert {d["device_class"] for d in kg if "device_class" in d} == {"switch"}
