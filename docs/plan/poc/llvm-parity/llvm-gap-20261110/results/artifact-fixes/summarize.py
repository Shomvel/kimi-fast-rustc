"""Recompute paired performance ratios from the artifact-fixes evidence archive.
Usage: python3 summarize.py RESULTS OUTPUT_JSON
"""
import json
import math
from pathlib import Path
import random
import statistics as s
import sys

root, output = map(Path, sys.argv[1:])
suite = root / 'suite'
rng = random.Random(20261110)

def ratios(a, b):
    values = [a[k] / b[k] for k in sorted(a.keys() & b.keys())]
    assert values and all(v > 0 for v in values)
    boots = sorted(s.median(rng.choices(values, k=len(values))) for _ in range(4000))
    return {'median': s.median(values), 'bootstrap_95': [boots[100], boots[3899]], 'pairs': len(values)}

result = {'runtime': [], 'compile': [], 'sizes': {}, 'tests': {}}
for crate in ['semver', 'base64', 'unicode-segmentation', 'humantime']:
    rec = json.loads((suite / 'runtime' / (crate + '.json')).read_text())
    for name, data in rec['workloads'].items():
        arms = {}
        for arm in ['llvm', 'baseline', 'transport']:
            samples = [v for v in data['samples'] if v['arm'] == arm and v['round'] >= 0]
            assert len(samples) == rec['rounds'] and all(v['exit_code'] == 0 for v in samples)
            arms[arm] = {v['round']: v['elapsed_ns'] for v in samples}
        assert len({v['checksum'] for v in data['samples']}) == 1
        profile = data['arms']
        result['runtime'].append({'crate': crate, 'workload': name,
            'candidate_over_baseline': ratios(arms['transport'], arms['baseline']),
            'candidate_over_llvm': ratios(arms['transport'], arms['llvm']),
            'instructions_over_baseline': profile['transport']['instructions'] / profile['baseline']['instructions'],
            'instructions_over_llvm': profile['transport']['instructions'] / profile['llvm']['instructions']})
    result['sizes'][crate] = {a: json.loads((suite/a/(crate+'.json')).read_text())['size']
                              for a in ['llvm','baseline','initial','layout','cleanup','wrappers','transport','ranges']}
    result['tests'][crate] = json.loads((suite/'transport'/(crate+'.json')).read_text())['tests']
rows = json.loads((suite/'compile-final.json').read_text())
assert all(r['exit_code'] == 0 and r['objects_identical_to_reference'] for r in rows)
for crate in result['sizes']:
    data = {arm: {r['round']:r for r in rows if r['crate']==crate and r['arm']==arm}
            for arm in ['baseline','transport']}
    values = {'crate': crate}
    for name, key in [('wall','wall_seconds'),('cpu',None),('rss','maxrss_kib'),('instructions','instructions')]:
        pairs = {a:{i:r[key] if key else r['user_seconds']+r['sys_seconds'] for i,r in rs.items()} for a,rs in data.items()}
        values[name] = ratios(pairs['transport'], pairs['baseline'])
    result['compile'].append(values)
result['runtime_geomean_ratio'] = math.exp(s.mean(math.log(r['candidate_over_baseline']['median']) for r in result['runtime']))
result['llvm_runtime_geomean_ratio'] = math.exp(s.mean(math.log(r['candidate_over_llvm']['median']) for r in result['runtime']))
result['compile_geomean_ratio'] = math.exp(s.mean(math.log(r['wall']['median']) for r in result['compile']))
result['gc_consumer'] = json.loads((root/'gc-consumers/summary.json').read_text())
result['method'] = {
    'runtime_candidate': 'transport',
    'compile_candidate': 'transport',
    'ranges': 'Static artifacts and correctness only; no equivalent paired performance run.',
    'bootstrap': '4000 resamples of round-paired ratios; median; seed 20261110; within-run uncertainty only.',
    'geomean': 'Equal weight per workload for runtime, per crate for compile; not a production traffic model.',
}
output.write_text(json.dumps(result,indent=2)+'\n')
print({k:v for k,v in result.items() if k.endswith('ratio')})
