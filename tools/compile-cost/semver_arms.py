#!/usr/bin/env python3
"""Compile semver with one kfrc backend under several counter arms (runs on E2B).

usage: python3 -I semver_arms.py --receipt build.command.json --backend backend.so
           --reference-objects DIR --out DIR [--arms total-check1,stages-check0,...]
           [--llvm-wrapper librustc_codegen_llvm_profile.so] [--env NAME=VALUE ...]

Arm names are <mode>-check<0|1>: mode `total` sets KFRC_PROFILE_TOTAL (one window
from codegen_crate entry through join_codegen), `stages` sets KFRC_PROFILE (one
window per stage and symbol), `plain` runs without counters, `perf` wraps the whole
rustc process in `perf stat -e instructions:u`. check0/check1 set KFRC_REGALLOC_CHECK.
Arm `llvm-total` builds with the LLVM profile wrapper (KFRC_LLVM_PROFILE).
Every kfrc arm extracts its objects and compares their sha256 with the reference.
"""
import argparse
import hashlib
import json
import os
import subprocess
from pathlib import Path

PERF = '/usr/lib/linux-tools/6.8.0-149-generic/perf'

parser = argparse.ArgumentParser()
parser.add_argument('--receipt', type=Path, required=True)
parser.add_argument('--backend', type=Path, required=True)
parser.add_argument('--reference-objects', type=Path, required=True)
parser.add_argument('--out', type=Path, required=True)
parser.add_argument('--arms', default='total-check1,total-check0,stages-check1,stages-check0')
parser.add_argument('--llvm-wrapper', type=Path)
parser.add_argument('--env', action='append', default=[])
parser.add_argument('--keep-objects', action='store_true')
args = parser.parse_args()
args.out.mkdir(parents=True, exist_ok=True)


def sha256(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def object_hashes(folder):
    return {path.name: sha256(path) for path in sorted(folder.glob('*.o'))}


# A missing reference directory is created from the first kfrc arm (baseline runs).
reference = object_hashes(args.reference_objects) if args.reference_objects.is_dir() else None
command = json.loads(args.receipt.read_text())['command']
command = [token for token in command if not token.startswith('-Zcodegen-backend=')]
base_env = {name: value for name, value in os.environ.items() if not name.startswith('KFRC_')}
base_env['RUSTC_BOOTSTRAP'] = '1'
for item in args.env:
    name, value = item.split('=', 1)
    base_env[name] = value


def run_arm(arm):
    folder = args.out / arm
    folder.mkdir(parents=True, exist_ok=True)
    for stale in ('total.jsonl', 'stages.jsonl', 'llvm.jsonl', 'perf.csv'):
        (folder / stale).unlink(missing_ok=True)
    env = dict(base_env)
    cmd = list(command)
    library = folder / 'libsemver.rlib'
    cmd[cmd.index('-o') + 1] = str(library)
    if arm == 'llvm-total':
        cmd.append('-Zcodegen-backend=' + str(args.llvm_wrapper))
        env['KFRC_LLVM_PROFILE'] = str(folder / 'llvm.jsonl')
    else:
        mode, check = arm.rsplit('-check', 1)
        cmd.append('-Zcodegen-backend=' + str(args.backend))
        env['KFRC_REGALLOC_CHECK'] = check
        if mode == 'total':
            env['KFRC_PROFILE_TOTAL'] = str(folder / 'total.jsonl')
        elif mode == 'stages':
            env['KFRC_PROFILE'] = str(folder / 'stages.jsonl')
        elif mode == 'perf':
            cmd = [PERF, 'stat', '-x,', '-e', 'instructions:u', '-o', str(folder / 'perf.csv'), '--'] + cmd
        else:
            assert mode == 'plain', arm
    with (folder / 'stdout.log').open('w') as stdout, (folder / 'stderr.log').open('w') as stderr:
        status = subprocess.run(cmd, env=env, stdout=stdout, stderr=stderr).returncode
    row = {'arm': arm, 'exit_code': status, 'command': cmd,
           'env': {k: v for k, v in env.items() if k.startswith('KFRC_')}}
    if status == 0:
        objects = folder / 'objects'
        objects.mkdir(exist_ok=True)
        for old in objects.iterdir():
            old.unlink()
        subprocess.run(['ar', 'x', str(library)], cwd=objects, check=True)
        hashes = object_hashes(objects)
        row['objects'] = len(hashes)
        row['object_sha256'] = hashes
        if arm != 'llvm-total':
            global reference
            if reference is None:
                args.reference_objects.mkdir(parents=True)
                for path in objects.glob('*.o'):
                    (args.reference_objects / path.name).write_bytes(path.read_bytes())
                reference = object_hashes(args.reference_objects)
                row['reference_created'] = True
            row['objects_identical_to_reference'] = hashes == reference
        if not args.keep_objects:
            for path in objects.iterdir():
                path.unlink()
            objects.rmdir()
            library.unlink()
    for name, key in (('total.jsonl', 'total_instructions'), ('llvm.jsonl', 'total_instructions')):
        path = folder / name
        if path.exists():
            records = [json.loads(line) for line in path.read_text().splitlines()]
            assert len(records) == 1 and records[0]['status'] == 'ok', records
            row[key] = records[0]['instructions']
    if (folder / 'stages.jsonl').exists():
        records = [json.loads(line) for line in (folder / 'stages.jsonl').read_text().splitlines()]
        bad = [record for record in records if record['status'] != 'ok']
        assert not bad, bad[:3]
        row['stage_window_sum'] = sum(record['instructions'] for record in records)
    if (folder / 'perf.csv').exists():
        for line in (folder / 'perf.csv').read_text().splitlines():
            fields = line.split(',')
            if len(fields) > 2 and fields[2].startswith('instructions'):
                row['process_instructions'] = int(fields[0])
    (folder / 'arm.json').write_text(json.dumps(row, indent=2) + '\n')
    summary = {k: v for k, v in row.items() if k not in ('command', 'object_sha256')}
    print(json.dumps(summary), flush=True)
    return row


rows = [run_arm(arm) for arm in args.arms.split(',') if arm]
(args.out / 'summary.json').write_text(json.dumps(
    {'backend': str(args.backend), 'backend_sha256': sha256(args.backend),
     'reference_objects': str(args.reference_objects),
     'arms': [{k: v for k, v in row.items() if k != 'object_sha256'} for row in rows]},
    indent=2) + '\n')
failed = [row['arm'] for row in rows if row['exit_code'] != 0 or row.get('objects_identical_to_reference') is False]
if failed:
    raise SystemExit(f'arms failed or changed objects: {failed}')
