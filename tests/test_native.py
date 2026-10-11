"""Portable rules and the native-only runtime boundary."""
import copy

import pytest
from helpers import curtain
from rule_data import call, rules

from tuya2ildevice import (
    Command,
    Connected,
    Message,
    SendMessage,
    Timer,
    TuyaDriver,
    Value,
)


def test_json_converter_reads_writes_and_keeps_state_across_ffi_calls():
    get = lambda *path: {'op': 'get', 'path': list(path)}
    converter = {
        'props': {'tens': {'type': 'number', 'rw': True, 'min': 0, 'max': 10},
                  'updates': {'type': 'number'}, 'tick': {'type': 'number'}},
        'initial_state': {'count': 0},
        'update': [
            {'set': ['state', 'count'], 'value': {'op': 'add', 'args': [get('state', 'count'), 1]}},
            {'emit': 'updates', 'value': get('state', 'count')},
            {'emit': 'tens', 'value': {'op': 'div', 'args': [get('codes', 'percent_control'), 10]}},
            {'timer': 'tick', 'after': 5}],
        'timer': [{'if': {'op': 'eq', 'args': [get('name'), 'tick']},
                   'then': [{'emit': 'tick', 'value': get('state', 'count')}]}],
        'write': {'tens': [{'send': {'code': 'percent_control',
                                    'value': {'op': 'mul', 'args': [get('value'), 10]}}}]}}
    driver = TuyaDriver(curtain(), overrides={'p': {'converters': {'declarative': converter}}}, rules=rules())
    driver.handle(0, Connected())
    outputs = driver.handle(1, Message('active', {'2': 40}))
    assert Value('tens', 4) in outputs and Value('updates', 1) in outputs
    assert driver.handle(2, Command('tens', 7)) == [SendMessage('set', {'dps': {'2': 70}})]
    assert Value('tick', 1) in driver.handle(6, Timer('c0:tick'))
    assert Value('updates', 2) in driver.handle(7, Message('active', {'2': 50}))


def test_rule_layout_changes_without_rebuilding_rust():
    pack = copy.deepcopy(rules())
    pack['entities']['switch']['layout'][0]['definition']['label'] = 'JSON configured switch'
    dev = {'id': 'd', 'category': 'kg', 'function': {'switch_1': {'type': 'Boolean', 'values': {}}}}
    result = call('create', rules=pack, device=dev, options={})
    assert result['driver']['assembly']['descriptor']['props']['switch_1']['label'] == 'JSON configured switch'


def test_the_package_has_no_python_conversion_code():
    import importlib
    for name in ('tuya2ildevice.tuya', 'tuya2ildevice.assemble', 'tuya2ildevice.fallback'):
        with pytest.raises(ImportError):
            importlib.import_module(name)
    driver = TuyaDriver(curtain(), rules=rules())
    driver.handle(0, Connected())
    assert Value('position', 30) in driver.handle(1, Message('state', {'3': 30}))
    assert driver.handle(2, Command('position', 50)) == [SendMessage('set', {'dps': {'2': 50}})]


def test_unsupported_rule_version_is_rejected():
    with pytest.raises(ValueError, match='unsupported rule version'):
        call('create', rules={'version': 99}, device=curtain(), options={})


def test_canonical_rule_pack_validates_against_its_schema():
    import json
    from pathlib import Path

    import jsonschema
    schema = json.loads((Path(__file__).resolve().parents[1] / 'rules/schema.json').read_text())
    jsonschema.validate(rules(), schema)
