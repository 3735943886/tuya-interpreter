"""Host-side fixture loading; production bindings never read rule files."""
import functools
import json
from pathlib import Path

from tuya2ildevice.native import call as native_call
from tuya2ildevice.native import register_rules


@functools.cache
def rules():
    return json.loads((Path(__file__).resolve().parents[1] / 'rules/00-default.json').read_text())


@functools.cache
def rule_id():
    return register_rules(rules())


def call(op, **payload):
    return native_call(op, **{'rules_id': rule_id(), **payload})
