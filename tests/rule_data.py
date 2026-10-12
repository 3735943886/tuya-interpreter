"""Host-side fixture loading; drivers never read rule files implicitly."""
import functools
import json
from pathlib import Path

from tuya_interpreter.native import call as native_call
from tuya_interpreter.native import register_rules


@functools.cache
def rules():
    return json.loads((Path(__file__).resolve().parents[1] / 'rules/builtin/00-default.json').read_text())


@functools.cache
def rule_id():
    return register_rules(rules())


def call(op, **payload):
    return native_call(op, **{'rules_id': rule_id(), **payload})
