#!/usr/bin/env python3
"""Run preserved legacy cases through real rustc, with explicit trap outcomes."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess

parser = argparse.ArgumentParser()
parser.add_argument('--corpus', type=Path, required=True)
parser.add_argument('--backend', type=Path, required=True)
parser.add_argument('--out', type=Path, required=True)
parser.add_argument('--arms', default='llvm,semantic-off,semantic-on')
a = parser.parse_args()
a.out.mkdir(parents=True, exist_ok=False)
env = os.environ.copy()
for key in ('KFRC_PROFILE', 'KFRC_PROFILE_TOTAL', 'KFRC_LLVM_PROFILE', 'KFRC_SPECIALIZATION_STATS', 'KFRC_INLINE', 'KFRC_OPTIMIZE', 'KFRC_DUMP_GRAPH', 'KFRC_DUMP_BODY', 'KFRC_INTERPRET'):
    env.pop(key, None)
env['RUSTC_BOOTSTRAP'] = '1'
flags = ['--edition=2021', '-Cpanic=abort', '-Copt-level=0', '-Ccodegen-units=1', '-Coverflow-checks=off', '-Cdebug-assertions=off', '-Zmir-opt-level=0']
summary = {'suites': [], 'scope': 'legacy behavior translated to actual Rust; LLVM reference and two sole semantic backend arms'}

def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()

def save():
    (a.out/'summary.json').write_text(json.dumps(summary, indent=2)+'\n')

def fail(message):
    summary['failure'] = message
    save()
    raise SystemExit(message)

def run(command, stem, active, expected='success'):
    with stem.with_suffix('.stdout').open('wb') as out, stem.with_suffix('.stderr').open('wb') as err:
        try:
            result = subprocess.run(command, stdout=out, stderr=err, env=active, timeout=60)
        except subprocess.TimeoutExpired:
            stem.with_suffix('.result.json').write_text(json.dumps({'command': command, 'timeout_seconds': 60})+'\n')
            fail(f'first failure: timeout {stem}')
    record = {'command': command, 'exit_code': result.returncode}
    stem.with_suffix('.result.json').write_text(json.dumps(record, indent=2)+'\n')
    if expected == 'success' and result.returncode != 0:
        fail(f'first failure: {stem} expected success, got {result.returncode}')
    if expected == 'trap' and result.returncode == 0:
        fail(f'first failure: {stem} expected trap, got success')
    return result.returncode, stem.with_suffix('.stdout').read_bytes()

manifests = sorted(a.corpus.glob('*/manifest.json'))
assert manifests, 'no suite manifests'
for manifest_path in manifests:
    source = manifest_path.parent
    manifest = json.loads(manifest_path.read_text())
    crate = manifest['crate_name']
    row = {'suite': source.name, 'source_manifest': manifest,
           'source_sha256': {f: digest(source/f) for f in ('lib.rs', 'driver.rs', 'manifest.json')}, 'arms': {}}
    summary['suites'].append(row)
    reference = {}
    for arm in a.arms.split(','):
        folder = a.out/source.name/arm
        folder.mkdir(parents=True)
        active = env.copy()
        backend_flags = []
        if arm != 'llvm':
            active['KFRC_OPTIMIZE'] = '1' if arm == 'semantic-on' else '0'
            backend_flags = ['-Zcodegen-backend='+str(a.backend)]
        library = folder/f'lib{crate}.rlib'
        run(['rustc', '+1.94.1', *list(dict.fromkeys([*flags, *manifest.get('rustc_flags', [])])), *backend_flags,
             '--crate-type=rlib', '--crate-name='+crate, str(source/'lib.rs'), '-o', str(library)], folder/'library', active)
        executable = folder/'driver'
        run(['rustc', '+1.94.1', *flags, str(source/'driver.rs'), '--extern='+crate+'='+str(library), '-o', str(executable)], folder/'driver-build', env)
        arm_row = {'library_sha256': digest(library), 'executable_sha256': digest(executable), 'runs': []}
        row['arms'][arm] = arm_row
        for case in manifest['runs']:
            got = run([str(executable), *case['run_args']], folder/('run-'+case['id']), env, case['expected_exit'])
            if case['expected_exit'] == 'success':
                assert got[1] == case['expected_stdout'].encode(), f'{source.name}/{arm}/{case["id"]}: expected output mismatch'
                if arm == 'llvm':
                    reference[case['id']] = got[1]
                else:
                    assert got[1] == reference[case['id']], f'{source.name}/{arm}/{case["id"]}: LLVM output mismatch'
            arm_row['runs'].append({'id': case['id'], 'exit_code': got[0], 'stdout_sha256': hashlib.sha256(got[1]).hexdigest()})
        save()
print(f'PASS: {len(manifests)} legacy suites, arms={a.arms}')
