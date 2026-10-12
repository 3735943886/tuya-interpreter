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
    filename = ('tuya_interpreter' if sys.platform == 'win32' else 'libtuya_interpreter') + suffix
    bundled = Path(__file__).parent / filename
    target = Path(__file__).resolve().parents[2] / 'rust/target'
    # In a checkout, the most recently built profile, so a stale build is never picked over a fresh one.
    builds = [b for b in (target / 'release' / filename, target / 'debug' / filename) if b.exists()]
    checkout = max(builds, key=lambda b: b.stat().st_mtime) if builds else target / 'release' / filename
    path = Path(os.environ['TUYA_ENGINE_LIBRARY']) if 'TUYA_ENGINE_LIBRARY' in os.environ else bundled if bundled.exists() else checkout
    lib = ctypes.CDLL(str(path))
    lib.tuya_engine_eval.argtypes = [ctypes.c_char_p]
    lib.tuya_engine_eval.restype = ctypes.c_void_p
    lib.tuya_engine_free.argtypes = [ctypes.c_void_p]
    lib.tuya_engine_free.restype = None
    return lib


def _encode(value):
    return json.dumps(value, ensure_ascii=False, allow_nan=False)


def _call(request):
    return _eval(_encode(request))


def _eval(text):
    lib = library()
    pointer = lib.tuya_engine_eval(text.encode())
    if not pointer:
        raise RuntimeError('Rust engine returned null')
    try:
        response = json.loads(ctypes.string_at(pointer))
    finally:
        lib.tuya_engine_free(pointer)
    if not response['ok']:
        raise ValueError(response['error'])
    return response['value']


_installed: set[str] = set()


def register_rules(pack):
    # Object order is executable rule data (platforms, bindings and programs).
    text = _encode(pack)
    identifier = hashlib.sha256(text.encode()).hexdigest()
    if identifier not in _installed:
        # The pack is encoded once, for both its ID and the request.
        _eval(f'{{"op": "install_rules", "rules_id": "{identifier}", "rules": {text}}}')
        _installed.add(identifier)
    return identifier


def unregister_rules(identifier):
    """Release a registered pack after its last driver stops using this ID."""
    _installed.discard(identifier)
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
