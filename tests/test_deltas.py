"""A delta dp (`report_type: sum`, a plug's `add_ele`): the property is the running total the driver adds up, and
`delta.accept_passive` (a block key, or the device's switch `delta_accept_passive`) counts passive reports too."""
import json

import pytest
from helpers import strat
from rule_data import rules

from tuya2ildevice import Command, Connected, Message, OverrideError, TuyaDriver, Value
from tuya2ildevice.io import SettingsChanged

PLUG = {"id": "plug1", "category": "cz", "product_id": "p", "name": "Plug", "function": {},
        "status_range": {"add_ele": {"code": "add_ele", "type": "Integer", "report_type": "sum",
                                     "values": json.dumps({"min": 0, "max": 50000, "scale": 3, "step": 1})}},
        "status": {}, "local_strategy": strat({"17": ("add_ele", "Integer")})}


def _plug(block=None, **kw):
    d = TuyaDriver(PLUG, overrides={"plug1": block} if block else None, **kw, rules=rules())
    d.handle(0, Connected())
    d.handle(1, Message("state", {"17": 84}))                           # a snapshot never adds
    return d


def _total(outs):
    return [o.value for o in outs if isinstance(o, Value) and o.prop == "add_ele"]


def test_by_default_only_a_push_adds():
    d = _plug()
    assert _total(d.handle(2, Message("passive", {"17": 5}))) == []
    assert _total(d.handle(3, Message("active", {"17": 5}))) == [0.005]
    assert _total(d.handle(4, Message("active", {"17": 5}))) == [0.010]   # the same increment again: added again


def test_accept_passive_adds_every_passive_report_even_a_repeated_value():
    d = _plug({"delta": {"accept_passive": True}})
    assert [_total(d.handle(n, Message("passive", {"17": 5}))) for n in (2, 3, 4)] == [[0.005], [0.010], [0.015]]
    assert _total(d.handle(5, Message("state", {"17": 5}))) == []      # the bridge's merged copy is not a report
    assert _total(d.handle(6, Message("active", {"17": 2}))) == [0.017]


def test_one_report_seen_twice_by_its_tuya_timestamp_adds_once():
    d = _plug({"delta": {"accept_passive": True}})
    assert _total(d.handle(2, Message("active", {"17": 5, "t": 100}))) == [0.005]
    assert _total(d.handle(3, Message("passive", {"17": 5, "t": 100}))) == []


def test_the_switch_saves_the_setting_and_only_a_device_with_a_delta_has_it():
    d = _plug(device_settings=True)
    assert d.descriptor["props"]["delta_accept_passive"] == {"type": "binary", "rw": True, "category": "config",
                                                             "label": "Count passive reports"}
    assert d.handle(2, Command("delta_accept_passive", True)) == [
        Value("delta_accept_passive", True), SettingsChanged({"delta": {"accept_passive": True}})]
    assert _total(d.handle(3, Message("passive", {"17": 5}))) == [0.005]  # in effect at once
    assert d.handle(4, Command("delta_accept_passive", False))[-1] == SettingsChanged({})
    assert "delta_accept_passive" not in TuyaDriver(PLUG, rules=rules()).descriptor["props"]          # no host to keep it
    no_delta = {**PLUG, "status_range": {}, "local_strategy": strat({"1": ("switch_1", "Boolean")})}
    assert "delta_accept_passive" not in TuyaDriver(no_delta, device_settings=True, rules=rules()).descriptor["props"]


@pytest.mark.parametrize("bad", [{"delta": {"accept_passive": 1}}, {"delta": {"passive": True}}, {"delta": True}])
def test_a_bad_delta_block_is_refused(bad):
    with pytest.raises(OverrideError):
        TuyaDriver(PLUG, overrides={"plug1": bad}, rules=rules())
