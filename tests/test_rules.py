"""Rule composition accepts decoded data in host-provided order."""
import copy

import pytest
from rule_data import rules

from tuya2ildevice import TuyaDriver, merge_rules
from tuya2ildevice.native import _call

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
