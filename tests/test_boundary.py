"""Interpretation works with in-memory data regardless of transport or rule paths."""
import builtins
from pathlib import Path

import pytest
from helpers import curtain
from rule_data import rules

import tuya_interpreter
from tuya_interpreter import (
    Command,
    Connected,
    Message,
    Reject,
    SendMessage,
    TuyaDriver,
    Value,
    merge_rules,
    preload,
)


def test_bidirectional_driver_does_not_read_rule_files(monkeypatch):
    pack = rules()
    preload()
    monkeypatch.setenv('TUYA_ENGINE_RULES', '/nonexistent/should-never-be-read')

    def unexpected(*args, **kwargs):
        raise AssertionError('runtime tried to read a file')

    monkeypatch.setattr(builtins, 'open', unexpected)
    monkeypatch.setattr(Path, 'read_text', unexpected)
    monkeypatch.setattr(Path, 'iterdir', unexpected)
    pack = merge_rules([{'overrides': {'p': {'device': {'label': 'Memory rules'}}}}], base=pack)['rules']
    driver = TuyaDriver(curtain(), rules=pack)
    assert driver.descriptor['label'] == 'Memory rules'
    driver.handle(0, Connected())
    assert Value('position', 30) in driver.handle(1, Message('state', {'3': 30}))
    assert driver.handle(2, Command('position', 50)) == [SendMessage('set', {'dps': {'2': 50}})]
    assert isinstance(driver.handle(3, Command('position', 101))[0], Reject)


def test_rules_are_required_and_transport_apis_are_absent():
    with pytest.raises(TypeError, match='rules'):
        TuyaDriver(curtain())
    for name in ('Hub', 'IlTopics', 'Publish', 'Subscribe', 'BridgeCommand', 'SaveSettings'):
        assert not hasattr(tuya_interpreter, name)


def test_device_identifier_is_not_validated_as_an_mqtt_topic():
    device = curtain()
    device['id'] = 'room/window'
    assert TuyaDriver(device, rules=rules()).descriptor['id'] == 'room/window'
