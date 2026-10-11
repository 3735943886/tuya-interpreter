"""The single rule format, native loading, and host reload boundary."""
import copy
import json

import pytest

from tuya2ildevice import Hub, TuyaDriver
from tuya2ildevice.native import _call, load_rules, merge_rule_files, rules

DEVICE = {'id': 'd', 'product_id': 'p', 'category': 'kg',
          'function': {'switch_1': {'type': 'Boolean', 'values': {}}}}


def write(path, value):
    path.write_text(json.dumps(value))


def test_native_default_and_ordered_user_layers(tmp_path):
    write(tmp_path / '20_user.json', {'overrides': {'p': {'device': {'label': 'later'}}}})
    write(tmp_path / '10_user.json', {'overrides': {'p': {'device': {'label': 'earlier', 'model': 'retained'}}}})
    (tmp_path / '.ignored.json').write_text('broken')
    (tmp_path / '_ignored.json').write_text('broken')
    (tmp_path / 'schema.json').write_text('broken')
    (tmp_path / 'nested').mkdir()
    (tmp_path / 'nested' / '30.json').write_text('broken')
    from pathlib import Path
    bundled = Path(__file__).resolve().parents[1] / 'rules'
    result = merge_rule_files([tmp_path, bundled])
    assert [Path(p).name for p in result['sources']] == ['00-default.json', '10_user.json', '20_user.json']
    driver = TuyaDriver(DEVICE, rules=result['rules'])
    assert driver.descriptor['label'] == 'later'
    assert driver.state['block']['device']['model'] == 'retained'
    assert result['rules']['platforms']


def test_array_replacement_member_deletion_and_null():
    base = copy.deepcopy(rules())
    base['maps']['custom'] = {'remove': 1, 'keep': 2, 'array': [1, 2]}
    merged = _call({'op': 'merge_rules', 'base': base, 'layers': [
        {'maps': {'custom': {'remove': {'$delete': True}, 'array': [3], 'keep': None}}}]})['rules']
    assert merged['maps']['custom'] == {'keep': None, 'array': [3]}
    assert base['maps']['custom']['remove'] == 1


def test_settings_follow_filename_order_without_special_priority(tmp_path):
    first, second = tmp_path / 'a', tmp_path / 'b'
    first.mkdir()
    second.mkdir()
    settings = first / 'zz_settings.json'
    write(settings, {'overrides': {'p': {'cover': {'settle': 9}}}})
    before = settings.read_bytes()
    write(second / 'zzz_user.json', {'overrides': {'p': {'cover': {'settle': 2}}}})
    assert load_rules([first, second])['overrides']['p']['cover']['settle'] == 2
    assert settings.read_bytes() == before


@pytest.mark.parametrize('value', [{'version': 2}, {'layouts': []}, {'unknown': {}},
                                 {'platforms': [{'platform': 'switch'}]}, {'version': {'$delete': True}}])
def test_invalid_rule_sets_are_rejected(tmp_path, value):
    write(tmp_path / '10_bad.json', value)
    with pytest.raises(ValueError):
        load_rules([tmp_path])


def test_rule_reload_detects_changes_without_descriptor_changes(tmp_path):
    hub = Hub([DEVICE])
    old = hub.drivers['d']
    write(tmp_path / '10_rule.json', {'maps': {'custom': {'a': 1}}})
    custom = load_rules([tmp_path])
    hub.reload({}, rules=custom)
    assert hub.drivers['d'] is not old
    assert hub.drivers['d'].descriptor == old.descriptor
    stable = hub.drivers['d']
    hub.reload({}, rules=custom)
    assert hub.drivers['d'] is stable


def test_no_implicit_default_or_default_filename_requirement(tmp_path):
    from pathlib import Path
    write(tmp_path / '50_complete.json', rules())
    result = merge_rule_files([tmp_path])
    assert result['rules'] == rules()
    assert [Path(p).name for p in result['sources']] == ['50_complete.json']
    with pytest.raises(ValueError):
        merge_rule_files([])


def test_sort_across_locations_and_validate_only_after_all_layers(tmp_path):
    first, second = tmp_path / 'a', tmp_path / 'b'
    first.mkdir()
    second.mkdir()
    base = copy.deepcopy(rules())
    platforms = base.pop('platforms')
    write(second / '00_base.json', base)
    write(first / '10_platforms.json', {'platforms': platforms})
    write(second / '30_label.json', {'overrides': {'p': {'device': {'label': 'last'}}}})
    write(first / '20_label.json', {'overrides': {'p': {'device': {'label': 'first'}}}})
    result = merge_rule_files([first, second])
    assert TuyaDriver(DEVICE, rules=result['rules']).descriptor['label'] == 'last'


def test_the_engine_reads_no_files_a_bad_file_is_the_hosts_report(tmp_path):
    from tuya2ildevice.native import load_rules_tolerant
    write(tmp_path / '10_good.json', {'overrides': {'p': {'device': {'label': 'good'}}}})
    (tmp_path / '20_broken.json').write_text('{broken')
    write(tmp_path / '30_bare.json', {'p': {}})
    with pytest.raises(ValueError, match='20_broken.json'):
        load_rules([tmp_path])
    loaded = load_rules_tolerant([tmp_path])
    assert [w.split(':')[0] for w in loaded['warnings']] == ['20_broken.json', '30_bare.json']
    assert loaded['rules']['overrides']['p']['device']['label'] == 'good'
    with pytest.raises(ValueError, match='unknown engine operation'):
        _call({'op': 'load_rules', 'paths': [str(tmp_path)]})
