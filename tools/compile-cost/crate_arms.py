#!/usr/bin/env python3
"""Compile each suite crate with one kfrc backend under counter arms (runs on E2B).

usage: python3 -I crate_arms.py --commands suite-commands.tsv --backend backend.so
           --reference-root DIR --out DIR [--arms total-check0,stages-check0]
           [--llvm-wrapper librustc_codegen_llvm_profile.so] [--crates a,b]

suite-commands.tsv: `<crate>\t<rustc command with -o @OUT@/lib.rlib>` (copied from
the 5-crate analysis, analysis-20261110/commands.tsv). Arms are those of
semver_arms.py (`total`, `stages`, `plain`, `perf` with check0/check1, and
`llvm-total`). Every kfrc arm compares its objects with <reference-root>/<crate>/,
which the first kfrc arm creates when it does not exist (baseline runs).
"""
import argparse
import hashlib
import json
import os
import shlex
import subprocess
from pathlib import Path

PERF = '/usr/lib/linux-tools/6.8.0-149-generic/perf'
parser = argparse.ArgumentParser()
parser.add_argument('--commands', type=Path, required=True)
parser.add_argument('--backend', type=Path, required=True)
parser.add_argument('--reference-root', type=Path, required=True)
parser.add_argument('--out', type=Path, required=True)
parser.add_argument('--arms', default='total-check0,stages-check0')
parser.add_argument('--llvm-wrapper', type=Path)
parser.add_argument('--crates')
args = parser.parse_args()


def sha256(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def object_hashes(folder):
    return {path.name: sha256(path) for path in sorted(folder.glob('*.o'))}


base_env = {name: value for name, value in os.environ.items() if not name.startswith('KFRC_')}
base_env['RUSTC_BOOTSTRAP'] = '1'
crates = []
for line in args.commands.read_text().splitlines():
    if line.strip():
        name, command = line.split('\t', 1)
        crates.append((name, shlex.split(command)))
if args.crates:
    wanted = args.crates.split(',')
    crates = [crate for crate in crates if crate[0] in wanted]

rows = []
for name, template in crates:
    reference_folder = args.reference_root / name
    reference = object_hashes(reference_folder) if reference_folder.is_dir() else None
    for arm in [arm for arm in args.arms.split(',') if arm]:
        folder = args.out / name / arm
        folder.mkdir(parents=True, exist_ok=True)
        for stale in ('total.jsonl', 'stages.jsonl', 'llvm.jsonl', 'perf.csv'):
            (folder / stale).unlink(missing_ok=True)
        command = [word.replace('@OUT@', str(folder)) for word in template]
        env = dict(base_env)
        if arm == 'llvm-total':
            command.append('-Zcodegen-backend=' + str(args.llvm_wrapper))
            env['KFRC_LLVM_PROFILE'] = str(folder / 'llvm.jsonl')
        else:
            mode, check = arm.rsplit('-check', 1)
            command.append('-Zcodegen-backend=' + str(args.backend))
            env['KFRC_REGALLOC_CHECK'] = check
            if mode == 'total':
                env['KFRC_PROFILE_TOTAL'] = str(folder / 'total.jsonl')
            elif mode == 'stages':
                env['KFRC_PROFILE'] = str(folder / 'stages.jsonl')
            elif mode == 'perf':
                command = [PERF, 'stat', '-x,', '-e', 'instructions:u', '-o', str(folder / 'perf.csv'), '--'] + command
            else:
                assert mode == 'plain', arm
        with (folder / 'stdout.log').open('w') as stdout, (folder / 'stderr.log').open('w') as stderr:
            status = subprocess.run(command, env=env, stdout=stdout, stderr=stderr).returncode
        row = {'crate': name, 'arm': arm, 'exit_code': status}
        library = folder / 'lib.rlib'
        if status == 0 and library.exists():
            objects = folder / 'objects'
            objects.mkdir(exist_ok=True)
            for old in objects.iterdir():
                old.unlink()
            subprocess.run(['ar', 'x', str(library)], cwd=objects, check=True)
            hashes = object_hashes(objects)
            row['objects'] = len(hashes)
            if arm != 'llvm-total':
                if reference is None:
                    reference_folder.mkdir(parents=True)
                    for path in objects.glob('*.o'):
                        (reference_folder / path.name).write_bytes(path.read_bytes())
                    reference = object_hashes(reference_folder)
                    row['reference_created'] = True
                row['objects_identical_to_reference'] = hashes == reference
            for path in objects.iterdir():
                path.unlink()
            objects.rmdir()
            library.unlink()
        for file in ('total.jsonl', 'llvm.jsonl'):
            if (folder / file).exists():
                records = [json.loads(text) for text in (folder / file).read_text().splitlines()]
                assert len(records) == 1 and records[0]['status'] == 'ok', records
                row['total_instructions'] = records[0]['instructions']
        if (folder / 'stages.jsonl').exists():
            records = [json.loads(text) for text in (folder / 'stages.jsonl').read_text().splitlines()]
            assert all(record['status'] == 'ok' for record in records)
            row['stage_window_sum'] = sum(record['instructions'] for record in records)
        if (folder / 'perf.csv').exists():
            for text in (folder / 'perf.csv').read_text().splitlines():
                fields = text.split(',')
                if len(fields) > 2 and fields[2].startswith('instructions'):
                    row['process_instructions'] = int(fields[0])
        (folder / 'arm.json').write_text(json.dumps(row, indent=2) + '\n')
        print(json.dumps(row), flush=True)
        rows.append(row)
(args.out / 'suite-summary.json').write_text(json.dumps(
    {'backend': str(args.backend), 'backend_sha256': sha256(args.backend), 'rows': rows}, indent=2) + '\n')
failed = [(row['crate'], row['arm']) for row in rows
          if row['exit_code'] != 0 or row.get('objects_identical_to_reference') is False]
if failed:
    raise SystemExit(f'arms failed or changed objects: {failed}')
