"""Converter effects follow the same transactional path from writes, packets and timers."""
import pytest
from helpers import curtain
from rule_data import rules

from tuya2ildevice import (
    CancelTimer,
    Command,
    Connected,
    Disconnected,
    Message,
    OverrideError,
    Reject,
    SendMessage,
    SetTimer,
    Timer,
    TuyaDriver,
    Value,
    merge_rules,
)


def driver(config):
    result = TuyaDriver(curtain(), rules=rules(), overrides={'p': {'converters': {'declarative': config}}})
    result.handle(0, Connected())
    return result


def test_write_can_publish_values_and_schedule_without_sending():
    d = driver({'props': {'x': {'type': 'number', 'rw': True}}, 'write': {'x': [
        {'emit': 'x', 'value': {'op': 'get', 'path': 'value'}}, {'timer': 'finish', 'after': 2}]},
        'timer': [{'emit': 'x', 'value': 0}]})
    assert d.handle(1, Command('x', 7)) == [Value('x', 7), SetTimer('c0:finish', 2)]
    assert d.handle(3, Timer('c0:finish')) == [Value('x', 0)]


def test_all_effects_work_from_write_update_and_timer():
    d = driver({'props': {'x': {'type': 'number', 'rw': True}},
                'write': {'x': [{'emit': 'x', 'value': 7}, {'timer': 'finish', 'after': 2},
                                {'send': {'code': 'percent_control', 'value': 50}}]},
                'update': [{'timer': 'finish', 'after': 1}, {'send': {'code': 'percent_control', 'value': 60}}],
                'timer': [{'emit': 'x', 'value': 0}, {'send': {'code': 'percent_control', 'value': 70}}]})
    assert d.handle(1, Command('x', 7)) == [Value('x', 7), SetTimer('c0:finish', 2),
                                         SendMessage('set', {'dps': {'2': 50}})]
    assert SendMessage('set', {'dps': {'2': 60}}) in d.handle(2, Message('active', {'3': 30}))
    assert d.handle(3, Timer('c0:finish')) == [Value('x', 0), SendMessage('set', {'dps': {'2': 70}})]


def test_rejected_send_rolls_back_state_values_and_timers():
    d = driver({'props': {'x': {'type': 'number', 'rw': True}}, 'initial_state': {'n': 0},
                'write': {'x': [{'set': 'state.n', 'value': 1}, {'emit': 'x', 'value': 7},
                                {'timer': 'finish', 'after': 2}, {'send': {'code': 'missing', 'value': 50}}]}})
    before = d.state.copy()
    assert d.handle(1, Command('x', 7)) == [Reject('x', 'unsupported', 'unsupported: no dp for missing')]
    assert d.state == before
    assert not d.timers


def test_failing_write_program_is_rejected_without_committing_state():
    d = driver({'props': {'x': {'type': 'number', 'rw': True}}, 'initial_state': {'n': 0},
                'write': {'x': [{'set': 'state.n', 'value': 1}, {'emit': 'x', 'value': {'op': 'error', 'message': 'boom'}}]}})
    before = d.state.copy()
    assert d.handle(1, Command('x', 7)) == [Reject('x', 'invalid_value', 'boom')]
    assert d.state == before


def test_failing_reset_program_still_disconnects_and_restores_initial_state():
    d = driver({'props': {'x': {'type': 'number'}}, 'initial_state': {'n': 0},
                'update': [{'set': 'state.n', 'value': 5}],
                'reset': [{'set': 'state.n', 'value': {'op': 'error', 'message': 'boom'}}]})
    d.handle(1, Message('active', {'3': 30}))
    assert Value('available', False) in d.handle(2, Disconnected())
    assert not d.linked
    assert d.state['machines'][0]['state'] == {'n': 0}


def test_timer_cancellation_and_hidden_properties_still_work():
    d = driver({'props': {'x': {'type': 'number', 'rw': True}},
                'update': [{'timer': 'finish', 'after': 2}],
                'write': {'x': [{'timer': 'finish', 'after': None}]}})
    d.handle(1, Message('state', {'3': 30}))
    assert d.handle(2, Command('x', 1)) == [CancelTimer('c0:finish')]
    assert d.handle(3, Timer('c0:finish')) == []


@pytest.mark.parametrize('block', [
    {'dp': []}, {'props': []}, {'remap': []}, {'converters': []},
    {'dp': {'1': None}}, {'props': {'x': None}}, {'remap': {'x': None}},
    {'category': 42}, {'device': {'label': 42}},
    {'props': {'position': {'hide': 'yes'}}},
    {'converters': {'declarative': {'props': {'x': {'type': 'banana'}}}}},
    {'converters': {'declarative': {'props': {'x': {'type': 'number', 'rw': 'yes'}}}}},
    {'converters': {'declarative': {'props': {'x': {'type': 'number', 'step': -1}}}}},
    {'converters': {'declarative': {'props': {'x': {'type': 'number'}}, 'update': {}}}},
    {'converters': {'declarative': {'props': {}, 'update': [{'timer': 'x', 'after': -1}]}}},
])
def test_malformed_overrides_fail_before_handling_packets(block):
    with pytest.raises(OverrideError):
        TuyaDriver(curtain(), rules=rules(), overrides={'p': block})
    with pytest.raises(ValueError):
        merge_rules([{'overrides': {'p': block}}], base=rules())
    merged = merge_rules([{'overrides': {'p': block}}], base=rules(), tolerant=True)
    assert merged['warnings'] and merged['rules'] == rules()


@pytest.mark.parametrize('expression', [
    {'op': 'range', 'args': [10**12]},
    {'op': 'hex', 'args': [1, 10**12]},
    {'op': 'replace', 'args': ['x' * 2000, 'x', 'y' * 2000]},
])
def test_generated_values_are_bounded_and_fail_transactionally(expression):
    d = driver({'props': {'x': {'type': 'number'}}, 'initial_state': {'n': 0},
                'update': [{'set': 'state.n', 'value': 1}, {'emit': 'x', 'value': expression}]})
    before = d.state.copy()
    with pytest.raises(ValueError, match='size limit'):
        d.handle(1, Message('active', {'3': 30}))
    assert d.state == before


def test_invalid_dynamic_timer_delay_is_rejected_transactionally():
    d = driver({'props': {}, 'update': [{'timer': 'x', 'after': {'op': 'get', 'path': 'codes.percent_state'}}]})
    before = d.state.copy()
    with pytest.raises(ValueError, match='timer delay'):
        d.handle(1, Message('active', {'3': -1}))
    assert d.state == before


def test_repeated_allocations_have_a_shared_budget():
    expression = {'op': 'map', 'args': [{'op': 'range', 'args': [5000]}], 'do': 'x' * 4000}
    d = driver({'props': {'x': {'type': 'text'}}, 'update': [{'emit': 'x', 'value': expression}]})
    before = d.state.copy()
    with pytest.raises(ValueError, match='allocation limit'):
        d.handle(1, Message('active', {'3': 30}))
    assert d.state == before


def test_converter_properties_are_checked_after_product_and_device_blocks_merge():
    d = TuyaDriver(curtain(), rules=rules(), overrides={
        'p': {'converters': {'declarative': {'props': {'x': {'type': 'number', 'rw': True}},
              'write': {'x': [{'send': {'code': 'percent_control', 'value': 50}}]}}}},
        'cur1': {'converters': {'declarative': {'props': {'x': {'label': 'Custom'}},
                 'update': [{'emit': 'x', 'value': 1}]}}}})
    d.handle(0, Connected())
    assert d.descriptor['props']['x']['label'] == 'Custom'
    assert Value('x', 1) in d.handle(1, Message('active', {'3': 30}))
    assert d.handle(2, Command('x', 2)) == [SendMessage('set', {'dps': {'2': 50}})]
    with pytest.raises(OverrideError, match='unknown property'):
        driver({'props': {}, 'update': [{'emit': 'unknown', 'value': 1}]})
