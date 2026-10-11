"""Compare native library revisions with identical Python/ctypes JSON transport costs.

Usage: python scripts/benchmark_abi.py /path/to/libtuya_rule_engine.so [iterations]
"""
import ctypes
import json
import statistics
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def main():
    library = ctypes.CDLL(sys.argv[1])
    library.tuya_engine_eval.argtypes = [ctypes.c_char_p]
    library.tuya_engine_eval.restype = ctypes.c_void_p
    library.tuya_engine_free.argtypes = [ctypes.c_void_p]
    library.tuya_engine_free.restype = None
    iterations = int(sys.argv[2]) if len(sys.argv) > 2 else 2000
    if iterations <= 0:
        raise ValueError('iterations must be positive')

    def call(request):
        pointer = library.tuya_engine_eval(json.dumps(request).encode())
        if not pointer:
            raise RuntimeError('null response')
        try:
            result = json.loads(ctypes.string_at(pointer))
        finally:
            library.tuya_engine_free(pointer)
        if not result['ok']:
            raise RuntimeError(result['error'])
        return result['value']

    call({'op': 'install_rules', 'rules_id': 'bench',
          'rules': json.loads((ROOT / 'rules/00-default.json').read_text())})
    cases = json.loads((ROOT / 'rust/engine/tests/driver_vectors.json').read_text())
    light = next(case for case in cases if case['name'] == 'dj_0gyaslysqfp4gfis')
    switch = {'name': 'switch', 'device': {'id': 'bench', 'category': 'kg',
              'function': {'switch_1': {'type': 'Boolean', 'values': {}}}},
              'options': {'dpmap': {'1': 'switch_1'}}}
    for case in (switch, light):
        samples = []
        for _ in range(5):
            state = call({'op': 'create', 'rules_id': 'bench', 'device': case['device'],
                          'options': case['options']})['driver']
            seed = next((step['input'] for step in case.get('steps', []) if step['input']['type'] == 'message'),
                        {'type': 'message', 'channel': 'state', 'json': {'1': False}})
            state = call({'op': 'handle', 'rules_id': 'bench', 'driver': state, 'input': seed, 'now': 0})['driver']
            for i in range(100):
                state = call({'op': 'handle', 'rules_id': 'bench', 'driver': state, 'now': i,
                              'input': {'type': 'message', 'channel': 'active', 'json': {'1': i % 2 == 0}}})['driver']
            start = time.perf_counter()
            for i in range(iterations):
                state = call({'op': 'handle', 'rules_id': 'bench', 'driver': state, 'now': i,
                              'input': {'type': 'message', 'channel': 'active', 'json': {'1': i % 2 == 0}}})['driver']
            samples.append((time.perf_counter() - start) * 1e6 / iterations)
        print(json.dumps({'case': case['name'], 'iterations': iterations, 'rounds': 5,
                          'json_ctypes_us': statistics.median(samples)}))


if __name__ == '__main__':
    main()
