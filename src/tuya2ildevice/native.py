"""Native library loading and JSON C ABI transport; no rule-file or device I/O."""
from __future__ import annotations

import ctypes
import functools
import hashlib
import json
import os
import sys
from pathlib import Path


@functools.cache
def library():
    suffix = '.dll' if sys.platform == 'win32' else '.dylib' if sys.platform == 'darwin' else '.so'
    filename = ('tuya_rule_engine' if sys.platform == 'win32' else 'libtuya_rule_engine') + suffix
    bundled = Path(__file__).parent / filename
    target = Path(__file__).resolve().parents[2] / 'rust/target'
    checkout = target / 'debug' / filename
    if not checkout.exists():
        checkout = target / 'release' / filename
    path = Path(os.environ['TUYA_ENGINE_LIBRARY']) if 'TUYA_ENGINE_LIBRARY' in os.environ else bundled if bundled.exists() else checkout
    lib = ctypes.CDLL(str(path))
    lib.tuya_engine_eval.argtypes = [ctypes.c_char_p]
    lib.tuya_engine_eval.restype = ctypes.c_void_p
    lib.tuya_engine_free.argtypes = [ctypes.c_void_p]
    lib.tuya_engine_free.restype = None
    return lib


def _call(request):
    lib = library()
    pointer = lib.tuya_engine_eval(json.dumps(request, ensure_ascii=False, allow_nan=False).encode())
    if not pointer:
        raise RuntimeError('Rust engine returned null')
    try:
        response = json.loads(ctypes.string_at(pointer))
    finally:
        lib.tuya_engine_free(pointer)
    if not response['ok']:
        raise ValueError(response['error'])
    return response['value']


def register_rules(pack):
    # Object order is executable rule data (platforms, bindings and programs).
    identifier = hashlib.sha256(json.dumps(pack, ensure_ascii=False, allow_nan=False).encode()).hexdigest()
    _call({'op': 'install_rules', 'rules_id': identifier, 'rules': pack})
    return identifier


def unregister_rules(identifier):
    """Release a registered pack after its last driver stops using this ID."""
    return _call({'op': 'remove_rules', 'rules_id': identifier})


def call(op, **payload):
    """Evaluate an operation using explicit request data; never load rules implicitly."""
    return _call({'op': op, **payload})


def merge_rules(layers, *, base=None, tolerant=False):
    """Compose decoded rule layers in caller order. File loading and ordering belong to the host."""
    return call('merge_rules', layers=list(layers), tolerant=tolerant, **({'base': base} if base is not None else {}))


def preload():
    """Load only the native library. Rule data is supplied by the caller."""
    library()
