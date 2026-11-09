#!/usr/bin/env python3
"""Measure matching complete backend windows, separately from timing runs.

The input command is a successful semver-perf build receipt. Each arm first
builds without counters, then proves profiled object bytes are identical.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import statistics
import subprocess

p = argparse.ArgumentParser()
p.add_argument('--compile-command', type=Path, required=True)
p.add_argument('--kfrc', type=Path, required=True)
p.add_argument('--llvm-wrapper', type=Path, required=True)
p.add_argument('--out', type=Path, required=True)
p.add_argument('--rounds', type=int, default=3)
a = p.parse_args()
assert a.rounds > 0
a.out.mkdir(parents=True, exist_ok=False)
command = json.loads(a.compile_command.read_text())['command']
command = command[next(i for i, token in enumerate(command) if Path(token).name == 'rustc'):]
command = [token for token in command if not token.startswith('-Zcodegen-backend=')]
clean = os.environ.copy()
for name in tuple(clean):
    if name.startswith('KFRC_') or name in ('RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS'):
        clean.pop(name, None)
clean['RUSTC_BOOTSTRAP'] = '1'

def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()

def build(arm, index):
    folder = a.out / f'{arm}-{index}'
    folder.mkdir()
    cmd = command.copy()
    library = folder / 'libsemver.rlib'
    cmd[cmd.index('-o') + 1] = str(library)
    env = clean.copy()
    if arm == 'kfrc':
        cmd.append('-Zcodegen-backend=' + str(a.kfrc))
        if index != 'reference':
            env['KFRC_PROFILE_TOTAL'] = str(folder / 'counter.jsonl')
    elif index != 'reference':
        cmd.append('-Zcodegen-backend=' + str(a.llvm_wrapper))
        env['KFRC_LLVM_PROFILE'] = str(folder / 'counter.jsonl')
    with (folder / 'stdout').open('w') as stdout, (folder / 'stderr').open('w') as stderr:
        result = subprocess.run(cmd, env=env, stdout=stdout, stderr=stderr, timeout=300)
    (folder / 'result.json').write_text(json.dumps({'command': cmd, 'exit_code': result.returncode}, indent=2) + '\n')
    if result.returncode:
        raise SystemExit(f'first failure: {folder}')
    objects = folder / 'objects'
    objects.mkdir()
    subprocess.run(['ar', 'x', str(library)], cwd=objects, check=True)
    hashes = {path.name: sha(path) for path in sorted(objects.glob('*.o'))}
    assert hashes, 'no object code found'
    row = {'object_sha256': hashes}
    if index != 'reference':
        records = [json.loads(line) for line in (folder / 'counter.jsonl').read_text().splitlines()]
        assert len(records) == 1 and records[0]['status'] == 'ok', 'counter unavailable or incomplete'
        counter = records[0]
        assert counter['instructions'] > 0
        assert counter['roots'] and all(root['time_enabled_ns'] == root['time_running_ns'] for root in counter['roots']), 'multiplexed counter'
        assert counter['stage'] == 'codegen_crate_through_join'
        row['counter'] = counter
    return row

summary = {
    'scope': 'codegen_crate entry through join_codegen; user instructions for initial process threads and descendants; includes collector and kfrc MIR conversion, excludes linking; not a MIR-conversion-excluded metric',
    'input_receipt': str(a.compile_command),
    'input_receipt_sha256': sha(a.compile_command),
    'backend_sha256': {'kfrc': sha(a.kfrc), 'llvm-wrapper': sha(a.llvm_wrapper)},
    'reference': {}, 'samples': [],
}
for arm in ('llvm', 'kfrc'):
    summary['reference'][arm] = build(arm, 'reference')
for index in range(a.rounds):
    for arm in (('llvm', 'kfrc') if index % 2 == 0 else ('kfrc', 'llvm')):
        row = build(arm, index)
        row.update(arm=arm, round=index)
        row['objects_identical_to_unprofiled'] = row['object_sha256'] == summary['reference'][arm]['object_sha256']
        assert row['objects_identical_to_unprofiled'], 'profiling changed object code'
        summary['samples'].append(row)
        (a.out / 'summary.json').write_text(json.dumps(summary, indent=2) + '\n')
summary['instruction_medians'] = {arm: statistics.median(row['counter']['instructions'] for row in summary['samples'] if row['arm'] == arm) for arm in ('llvm', 'kfrc')}
summary['kfrc_over_llvm'] = summary['instruction_medians']['kfrc'] / summary['instruction_medians']['llvm']
(a.out / 'summary.json').write_text(json.dumps(summary, indent=2) + '\n')
print(json.dumps({k: v for k, v in summary.items() if k not in ('reference', 'samples')}, indent=2))
