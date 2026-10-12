"""Compare native library revisions with identical Python/ctypes JSON transport costs.

Measures both the explicit-state transport (`handle`, the whole driver on each message) and
engine-held drivers (`open`/`drive`, only the input and outputs on each message).

Usage: python scripts/benchmark_abi.py /path/to/libtuya_interpreter.so [iterations]
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
          'rules': json.loads((ROOT / 'rules/builtin/00-default.json').read_text())})
    cases = json.loads((ROOT / 'rust/engine/tests/driver_vectors.json').read_text())
    light = next(case for case in cases if case['name'] == 'dj_0gyaslysqfp4gfis')
    switch = {'name': 'switch', 'device': {'id': 'bench', 'category': 'kg',
              'function': {'switch_1': {'type': 'Boolean', 'values': {}}}},
              'options': {'dpmap': {'1': 'switch_1'}}}
    def message(i):
        return {'type': 'message', 'channel': 'active', 'json': {'1': i % 2 == 0}}

    def explicit_state(case, seed):
        state = call({'op': 'create', 'rules_id': 'bench', 'device': case['device'],
                      'options': case['options']})['driver']
        state = call({'op': 'handle', 'rules_id': 'bench', 'driver': state, 'input': seed, 'now': 0})['driver']

        def step(i):
            nonlocal state
            state = call({'op': 'handle', 'rules_id': 'bench', 'driver': state, 'now': i,
                          'input': message(i)})['driver']
        return step

    def engine_held(case, seed):
        handle = call({'op': 'open', 'rules_id': 'bench', 'device': case['device'],
                       'options': case['options']})['handle']
        call({'op': 'drive', 'handle': handle, 'input': seed, 'now': 0})
        return lambda i: call({'op': 'drive', 'handle': handle, 'now': i, 'input': message(i)})

    for case in (switch, light):
        seed = next((step['input'] for step in case.get('steps', []) if step['input']['type'] == 'message'),
                    {'type': 'message', 'channel': 'state', 'json': {'1': False}})
        result = {'case': case['name'], 'iterations': iterations, 'rounds': 5}
        for name, start_driver in (('json_ctypes_us', explicit_state), ('handle_ctypes_us', engine_held)):
            samples = []
            for _ in range(5):
                step = start_driver(case, seed)
                for i in range(100):
                    step(i)
                start = time.perf_counter()
                for i in range(iterations):
                    step(i)
                samples.append((time.perf_counter() - start) * 1e6 / iterations)
            result[name] = statistics.median(samples)
        print(json.dumps(result))

if __name__ == '__main__':
    main()
