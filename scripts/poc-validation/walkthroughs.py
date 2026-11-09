#!/usr/bin/env python3
"""Compile unchanged W01-W10 programs and compare LLVM/sole semantic paths.

This proves behavior and records graph structure; it does not establish the
unimplemented R cache/answer-key model or pass visit counters.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess

p = argparse.ArgumentParser()
p.add_argument('--inputs', type=Path, required=True)
p.add_argument('--backend', type=Path, required=True)
p.add_argument('--out', type=Path, required=True)
p.add_argument('--cases', default='01,02,03,04,05,06,07,08,09,10')
p.add_argument('--arms', default='llvm,semantic-off,semantic-on')
a = p.parse_args()
a.out.mkdir(parents=True, exist_ok=False)
env = os.environ.copy()
for name in ('KFRC_PROFILE', 'KFRC_PROFILE_TOTAL', 'KFRC_LLVM_PROFILE', 'KFRC_SPECIALIZATION_STATS', 'KFRC_INLINE', 'KFRC_OPTIMIZE', 'KFRC_DUMP_GRAPH', 'KFRC_DUMP_BODY', 'KFRC_INTERPRET'):
    env.pop(name, None)
env['RUSTC_BOOTSTRAP'] = '1'
flags = ['--edition=2021', '-Cpanic=abort', '-Copt-level=0', '-Coverflow-checks=off', '-Cdebug-assertions=off', '-Ccodegen-units=1', '-Zmir-opt-level=0']
summary = {'cases': {}, 'pass_visits': None, 'pass_visits_limitation': 'candidate01 does not instrument node visits; node/rewrite counts are not visit counts', 'scope': 'original walkthrough sources; panic=abort; behavior and S/L structure only; W08 baseline is not incremental-cache validation'}

def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()

def run(command, stem, arm):
    active = env.copy()
    if arm.startswith('semantic-'):
        active.update(KFRC_OPTIMIZE='1' if arm.endswith('on') else '0', KFRC_DUMP_GRAPH='1')
    with stem.with_suffix('.stdout').open('w') as out, stem.with_suffix('.stderr').open('w') as err:
        result = subprocess.run(command, env=active, stdout=out, stderr=err)
    record = {'command': command, 'exit_code': result.returncode, 'arm': arm}
    stem.with_suffix('.result.json').write_text(json.dumps(record, indent=2)+'\n')
    if result.returncode:
        summary['stopped_at'] = str(stem)
        (a.out/'summary.json').write_text(json.dumps(summary, indent=2)+'\n')
        raise SystemExit(f'first failure; stop before repeating class: {stem}')
    return stem.with_suffix('.stdout').read_bytes()

for case in a.cases.split(','):
    case_dir = a.out/f'w{case}'
    case_dir.mkdir()
    sources = [a.inputs/f'w{case}.rs'] if case != '08' else [a.inputs/'w08/lib.rs', a.inputs/'w08/bin.rs']
    row = {'source_sha256': {str(s.relative_to(a.inputs)):sha(s) for s in sources}, 'arms': {}}
    summary['cases'][case] = row
    reference = None
    for arm in a.arms.split(','):
        folder = case_dir/arm
        folder.mkdir()
        backend_flags = [] if arm == 'llvm' else ['-Zcodegen-backend='+str(a.backend)]
        dep_flags = []
        if case == '08':
            lib = folder/'libw08lib.rlib'
            run(['rustc', '+1.94.1', *flags, *backend_flags, '--crate-name=w08lib', '--crate-type=rlib', str(sources[0]), '-o', str(lib)], folder/'lib-build', arm)
            dep_flags = ['--extern=w08lib='+str(lib)]
        executable = folder/'program'
        run(['rustc', '+1.94.1', *flags, *backend_flags, *dep_flags, str(sources[-1]), '-o', str(executable)], folder/'build', arm)
        output = run([str(executable)], folder/'execute', arm)
        if reference is None:
            reference = output
        assert output == reference, f'{case}/{arm}: output differs from LLVM'
        structures = []
        for log in folder.glob('*build.stderr'):
            text = log.read_text()
            for m in re.finditer(r'^graph ([^\n]+) \((Specialized|Layout), (\w+)\) \{\n(.*?)^\}', text, re.M|re.S):
                symbol, level, state, body = m.groups()
                ops = re.findall(r'^  n\d+ = ([A-Za-z]+)', body, re.M)
                structures.append({'symbol':symbol, 'level':level, 'state':state, 'nodes':len(ops), 'opcodes':{op:ops.count(op) for op in sorted(set(ops))}})
        row['arms'][arm] = {'executable_sha256':sha(executable), 'stdout_sha256':hashlib.sha256(output).hexdigest(), 'output_bytes':len(output), 'graphs':structures}
    (a.out/'summary.json').write_text(json.dumps(summary, indent=2)+'\n')
print(f'PASS: unchanged walkthrough behavior, arms={a.arms}; see exact structural counts and scope limits in summary.json')
