#!/usr/bin/env python3
"""Interleaved whole-rustc instruction, time and RSS measurements on E2B."""
import argparse
import hashlib
import json
import os
import shlex
import statistics
import subprocess
from pathlib import Path

parser = argparse.ArgumentParser()
parser.add_argument('--commands', type=Path, required=True)
parser.add_argument('--base', type=Path, required=True)
parser.add_argument('--candidate', type=Path, required=True)
parser.add_argument('--reference-root', type=Path, required=True)
parser.add_argument('--out', type=Path, required=True)
args = parser.parse_args()
args.out.mkdir(parents=True, exist_ok=True)
env = {k: v for k, v in os.environ.items() if not k.startswith('KFRC_')}
env.update(RUSTC_BOOTSTRAP='1', KFRC_REGALLOC_CHECK='0')
perf = '/usr/lib/linux-tools/6.8.0-149-generic/perf'
backends = {'base': args.base, 'candidate': args.candidate, 'llvm': None}
orders = [('base', 'candidate', 'llvm'), ('llvm', 'candidate', 'base'),
          ('base', 'llvm', 'candidate')]
sha = lambda path: hashlib.sha256(path.read_bytes()).hexdigest()
rows = []
inputs = {'backends': {k: sha(v) if v else 'rustc LLVM' for k, v in backends.items()},
          'orders': orders, 'checker': False, 'scope': 'whole rustc process',
          'toolchain': subprocess.check_output(['rustc', '+1.94.1', '-Vv'], text=True),
          'load_before': Path('/proc/loadavg').read_text().strip()}
(args.out / 'inputs.json').write_text(json.dumps(inputs, indent=2) + '\n')
for line in args.commands.read_text().splitlines():
    name, raw = line.split('\t', 1)
    reference = {p.name: sha(p) for p in (args.reference_root / name).glob('*.o')}
    assert reference, ('missing reference', name)
    for round_index, order in enumerate(orders):
        for arm in order:
            folder = args.out / name / str(round_index) / arm
            folder.mkdir(parents=True)
            command = [word.replace('@OUT@', str(folder)) for word in shlex.split(raw)]
            if backends[arm]:
                command.append('-Zcodegen-backend=' + str(backends[arm]))
            measured = ['/usr/bin/time', '-v', '-o', str(folder / 'time.txt'),
                        perf, 'stat', '-x,', '-e', 'instructions:u,cycles:u',
                        '-o', str(folder / 'perf.csv'), '--', *command]
            with (folder / 'stdout').open('w') as out, (folder / 'stderr').open('w') as err:
                status = subprocess.run(measured, env=env, stdout=out, stderr=err).returncode
            record = {'crate': name, 'round': round_index, 'arm': arm, 'exit_code': status,
                      'command': command, 'load': Path('/proc/loadavg').read_text().strip()}
            (folder / 'command.json').write_text(json.dumps(record, indent=2) + '\n')
            assert status == 0, record
            for text in (folder / 'perf.csv').read_text().splitlines():
                fields = text.split(',')
                if len(fields) > 2 and fields[2] in ('instructions:u', 'cycles:u'):
                    record[fields[2].split(':')[0]] = int(fields[0])
            for text in (folder / 'time.txt').read_text().splitlines():
                key, _, value = text.strip().partition(': ')
                if key.startswith('Elapsed (wall clock)'):
                    parts = [float(p) for p in value.split(':')]
                    record['wall_seconds'] = sum(p * 60 ** i for i, p in enumerate(reversed(parts)))
                elif key == 'User time (seconds)':
                    record['user_seconds'] = float(value)
                elif key == 'System time (seconds)':
                    record['system_seconds'] = float(value)
                elif key == 'Maximum resident set size (kbytes)':
                    record['max_rss_bytes'] = int(value) * 1024
            objects = folder / 'objects'
            objects.mkdir()
            subprocess.run(['ar', 'x', str(folder / 'lib.rlib')], cwd=objects, check=True)
            if backends[arm]:
                record['objects_identical'] = {p.name: sha(p) for p in objects.glob('*.o')} == reference
                assert record['objects_identical'], record
            for path in objects.iterdir():
                path.unlink()
            objects.rmdir()
            (folder / 'lib.rlib').unlink()
            (folder / 'result.json').write_text(json.dumps(record, indent=2) + '\n')
            rows.append(record)
            print(json.dumps({k: v for k, v in record.items() if k != 'command'}), flush=True)
summary = []
for name in dict.fromkeys(r['crate'] for r in rows):
    for arm in backends:
        samples = [r for r in rows if r['crate'] == name and r['arm'] == arm]
        summary.append({'crate': name, 'arm': arm, **{
            field: {'median': statistics.median(r[field] for r in samples),
                    'min': min(r[field] for r in samples),
                    'max': max(r[field] for r in samples)}
            for field in ('instructions', 'cycles', 'wall_seconds', 'user_seconds',
                          'system_seconds', 'max_rss_bytes')}})
(args.out / 'summary.json').write_text(json.dumps(
    {'inputs': inputs, 'rows': summary, 'load_after': Path('/proc/loadavg').read_text().strip()},
    indent=2) + '\n')
