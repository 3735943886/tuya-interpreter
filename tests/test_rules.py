"""Rule composition accepts decoded data in host-provided order."""
import copy

import pytest
from rule_data import rules

from tuya2ildevice import Connected, Message, TuyaDriver, Value, merge_rules
from tuya2ildevice.native import _call, register_rules

DEVICE = {'id': 'd', 'product_id': 'p', 'category': 'kg',
          'function': {'switch_1': {'type': 'Boolean', 'values': {}}}}


def test_ordered_layers_and_descriptor():
    result = merge_rules([rules(),
                          {'overrides': {'p': {'device': {'label': 'earlier', 'model': 'kept'}}}},
                          {'overrides': {'p': {'device': {'label': 'later'}}}}])
    driver = TuyaDriver(DEVICE, rules=result['rules'])
    assert driver.descriptor['label'] == 'later'
    assert driver.state['block']['device']['model'] == 'kept'


def test_array_replacement_member_deletion_and_null():
    base = copy.deepcopy(rules())
    base['maps']['custom'] = {'remove': 1, 'keep': 2, 'array': [1, 2]}
    merged = merge_rules([{'maps': {'custom': {'remove': {'$delete': True}, 'array': [3], 'keep': None}}}],
                         base=base)['rules']
    assert merged['maps']['custom'] == {'keep': None, 'array': [3]}
    assert base['maps']['custom']['remove'] == 1


@pytest.mark.parametrize('value', [{'version': 2}, {'layouts': []}, {'unknown': {}},
                                 {'platforms': [{'platform': 'switch'}]}, {'version': {'$delete': True}}])
def test_invalid_rule_sets_are_rejected(value):
    with pytest.raises(ValueError):
        merge_rules([value], base=rules())


def test_validate_after_all_layers_and_no_implicit_default():
    base = copy.deepcopy(rules())
    platforms = base.pop('platforms')
    assert merge_rules([base, {'platforms': platforms}])['rules'] == rules()
    with pytest.raises(ValueError):
        merge_rules([])


def test_tolerant_layers_and_removed_operations():
    result = merge_rules([{'source': 'good', 'rules': {'overrides': {'p': {'device': {'label': 'good'}}}}},
                          {'source': 'bad', 'rules': {'unknown': {}}}], base=rules(), tolerant=True)
    assert result['rules']['overrides']['p']['device']['label'] == 'good'
    assert result['warnings']
    for op in ('load_rules', 'hub', 'il'):
        with pytest.raises(ValueError, match='unknown engine operation'):
            _call({'op': op})


def ordered_packs():
    first = copy.deepcopy(rules())
    first['entities']['switch']['state'] = {
        'bind': {'a': True, 'b': {'op': 'get', 'path': 'a', 'default': False}},
        'fields': {'is_on': {'op': 'get', 'path': 'b'}}}
    second = copy.deepcopy(first)
    second['entities']['switch']['state']['bind'] = dict(
        reversed(list(second['entities']['switch']['state']['bind'].items())))
    return first, second


def test_registered_rules_preserve_executable_object_order():
    first, second = ordered_packs()
    assert register_rules(first) != register_rules(second)
    for pack, expected in ((first, True), (second, False)):
        driver = TuyaDriver(DEVICE, rules=pack, dpmap={'1': 'switch_1'})
        driver.handle(0, Connected())
        assert Value('switch_1', expected) in driver.handle(1, Message('state', {'1': True}))


def test_same_registration_id_cannot_hide_an_order_change():
    first, second = ordered_packs()
    _call({'op': 'install_rules', 'rules_id': 'order-sensitive-test', 'rules': first})
    with pytest.raises(ValueError, match='different content'):
        _call({'op': 'install_rules', 'rules_id': 'order-sensitive-test', 'rules': second})


@pytest.mark.parametrize('identifier', [None, '', 123])
def test_registration_requires_a_nonempty_string_id(identifier):
    with pytest.raises(ValueError, match='nonempty string'):
        _call({'op': 'install_rules', 'rules_id': identifier, 'rules': rules()})


def test_handle_requires_the_same_registered_or_inline_rules():
    first, second = ordered_packs()
    driver = TuyaDriver(DEVICE, rules=first)
    request = {'op': 'handle', 'driver': driver.state, 'now': 0, 'input': {'type': 'connected'}}
    for context in ({}, {'rules_id': 'not-registered'}, {'rules_id': register_rules(second)},
                    {'rules': second}):
        with pytest.raises(ValueError, match='rules'):
            _call({**request, **context})
    assert _call({**request, 'rules': first})['driver']['linked']
    inline = _call({'op': 'create', 'device': DEVICE, 'options': {}, 'rules': first})['driver']
    with pytest.raises(ValueError, match='do not match'):
        _call({**request, 'driver': inline, 'rules': second})
    assert _call({**request, 'driver': inline, 'rules_id': register_rules(first)})['driver']['linked']


@pytest.mark.parametrize('context', [{}, {'rules_id': 'not-registered'}, {'rules': {'version': 1}}])
def test_create_rejects_missing_or_incomplete_rules(context):
    with pytest.raises(ValueError, match='rules'):
        _call({'op': 'create', 'device': DEVICE, 'options': {}, **context})


def test_inline_rules_do_not_inherit_an_unrelated_registration_id():
    driver = _call({'op': 'create', 'device': DEVICE, 'options': {},
                    'rules': rules(), 'rules_id': 'not-registered'})['driver']
    assert driver['rules_id'] == ''


def test_rule_independent_operations_need_no_pack():
    assert _call({'op': 'merge', 'mappings': [{'x': 1}]}) == {'x': 1}
    driver = TuyaDriver(DEVICE, rules=rules())
    assert _call({'op': 'settings', 'driver': driver.state}) == driver.settings_block()
