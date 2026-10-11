"""Check the Rust C ABI against frozen full-driver Python traces."""
import json
import sys
from pathlib import Path
ROOT=Path(__file__).resolve().parents[2]
sys.path.insert(0,str(ROOT/'src'))
from tuya2ildevice.native import call


def canonical(outputs):
    # Rejection codes are the portable contract; human diagnostic wording may differ.
    return [{k:v for k,v in out.items() if not (out.get('type')=='reject' and k=='reason')} for out in outputs]

rules=json.loads((ROOT/'rules/00-default.json').read_text())
failures=[]
cases=json.loads((ROOT/'rust/engine/tests/driver_vectors.json').read_text())
for case in cases:
    try:
        driver=call('create',rules=rules,device=case['device'],options=case['options'])['driver']
        if driver['assembly']['descriptor']!=case['descriptor']:
            failures.append((case['name'],'descriptor',driver['assembly']['descriptor'],case['descriptor']))
            continue
        for step in case['steps']:
            result=call('handle',rules=rules,driver=driver,input=step['input'],now=step['now'])
            driver=result['driver']
            if canonical(result['outputs'])!=canonical(step['outputs']):
                failures.append((case['name'],step['input'],result['outputs'],step['outputs']))
                break
    except Exception as e:
        failures.append((case['name'],'exception',str(e),None))
print(f'{len(cases)-len(failures)}/{len(cases)} full-driver traces passed')
path=ROOT/'rust/target/driver_diff.json'
path.write_text(json.dumps(failures,ensure_ascii=False,indent=2)+'\n')
for fail in failures[:8]:
    print(fail[0],str(fail[1])[:100],str(fail[2])[:250],str(fail[3])[:250])
if failures: print(f'Differences: {path}');sys.exit(1)
