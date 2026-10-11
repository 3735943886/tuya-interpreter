"""C ABI transport, and the rule files read for the engine (which reads none itself); no conversion rules."""
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


def rule_paths(paths=(), *, bundled_path=None):
    """The bundled rule directory, then `paths`."""
    bundled = Path(__file__).parent / 'rules'
    checkout = Path(__file__).resolve().parents[2] / 'rules'
    location = bundled_path or (bundled if bundled.is_dir() else checkout)
    return [Path(location), *[Path(p) for p in paths]]


def rule_files(locations):
    """Every rule file of `locations` (a file, or a directory's visible `*.json` but `schema.json`, not recursing), in
    filename byte order; equal names keep the location order."""
    files = []
    for location in map(Path, locations):
        if location.is_file():
            files.append(location)
        elif location.is_dir():
            files += [p for p in location.iterdir()
                      if p.is_file() and p.suffix == '.json' and p.name != 'schema.json' and p.name[0] not in '._']
    return sorted(files, key=lambda p: os.fsencode(p.name))


def merge_rule_files(locations, *, tolerant=False):
    """Read the rule files of `locations` and merge them in the engine: `{rules, sources, warnings}`. With `tolerant`,
    a file that cannot be read or would make the rules invalid is reported in `warnings` and left out."""
    files = rule_files(locations)
    layers, warnings = [], []
    for path in files:
        try:
            layers.append({'source': str(path), 'rules': json.loads(path.read_text(encoding='utf-8'))})
        except (OSError, ValueError) as error:
            if not tolerant:
                raise ValueError(f'{path}: {error}') from error
            warnings.append(f'{path.name}: {error}')
    result = _call({'op': 'merge_rules', 'layers': layers, 'tolerant': tolerant})
    names = {str(p): p.name for p in files}

    def short(warning):
        source, _, message = warning.partition(': ')
        return f'{names.get(source, source)}: {message}'
    result['warnings'] = warnings + [short(w) for w in result['warnings']]
    return result


def load_rules(paths=(), *, bundled_path=None):
    """The bundled rules with the rule files of `paths` over them."""
    return merge_rule_files(rule_paths(paths, bundled_path=bundled_path))['rules']


def load_rules_tolerant(paths=(), *, bundled_path=None):
    """Like `load_rules`, but a file that does not load is left out: `{rules, sources, warnings}`."""
    return merge_rule_files(rule_paths(paths, bundled_path=bundled_path), tolerant=True)


@functools.cache
def rules():
    paths = [p for p in os.environ.get('TUYA_ENGINE_RULES', '').split(os.pathsep) if p]
    return load_rules(paths)


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
    identifier = hashlib.sha256(json.dumps(pack, sort_keys=True).encode()).hexdigest()
    _call({'op': 'install_rules', 'rules_id': identifier, 'rules': pack})
    return identifier


@functools.cache
def rule_id():
    return register_rules(rules())


def call(op, **payload):
    return _call({'op': op, 'rules_id': rule_id(), **payload})


def preload():
    library()
    rule_id()
