#!/usr/bin/env python3
"""Replay captured target invocations without replacing LLVM dependencies."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import time

p = argparse.ArgumentParser()
p.add_argument('--project', required=True)
p.add_argument('--backend', type=Path, required=True)
p.add_argument('--label', default='survey-v1')
p.add_argument('--reuse-complete-from', type=Path)
p.add_argument('--unit-timeout', type=int, default=7200)
p.add_argument('--cranelift-opt', choices=['none','speed','speed_and_size'])
p.add_argument('--unit')
p.add_argument('--skip-unit')
p.add_argument('--env-supplement', type=Path)
args = p.parse_args()
supplements = json.loads(args.env_supplement.read_text()) if args.env_supplement else {}
root = Path('/workspace/bigcrates')
out = root / 'surveys' / args.label / args.project
out.mkdir(parents=True, exist_ok=True)
results = []
excluded = []
captures = sorted((root / 'captures' / args.project).glob('*.json'))
for capture in captures:
    record = json.loads(capture.read_text())
    argv = record['argv']
    emits = [a.split('=', 1)[1].split(',') for a in argv if a.startswith('--emit=')]
    if '--target' not in argv or '--crate-name' not in argv or not any('link' in emit for emit in emits):
        excluded.append({'capture': str(capture), 'argv': argv, 'exit_code': record.get('exit_code'),
                         'reason': 'not a formal target link-emitting unit'})
        (out / 'excluded-probes.json').write_text(json.dumps(excluded, indent=2) + '\n')
        continue
    if argv[argv.index('--target') + 1] != 'x86_64-unknown-linux-gnu':
        continue
    if record.get('exit_code') != 0:
        raise SystemExit(f'Cannot replay unsuccessful LLVM input: {capture}')
    crate = argv[argv.index('--crate-name') + 1]
    if args.unit and crate != args.unit:
        continue
    if args.skip_unit and crate in args.skip_unit.split(','):
        continue
    if args.reuse_complete_from:
        prior_unit = args.reuse_complete_from / args.project / (crate + '-' + capture.stem.split('-')[-1])
        prior_result = prior_unit / 'result.json'
        if prior_result.exists():
            prior = json.loads(prior_result.read_text())
            prior_complete = bool(prior['reports']) and all(
                any(s.get('status') == 'complete' for s in r['summary']) for r in prior['reports'])
            if prior_complete:
                results.append({'crate': crate, 'unit': str(prior_unit), 'exit_code': prior['exit_code'],
                                'seconds': prior['seconds'], 'reports': prior['reports'],
                                'reused_from': str(args.reuse_complete_from)})
                (out / 'index.json').write_text(json.dumps(results, indent=2) + '\n')
                print(f'SURVEY_REUSED_COMPLETE {args.project} {crate}', flush=True)
                continue
    unit = out / (crate + '-' + capture.stem.split('-')[-1])
    unit.mkdir(exist_ok=True)
    (unit / 'objects').mkdir(exist_ok=True)
    report = unit / 'reports'
    report.mkdir(exist_ok=True)
    replay = []
    i = 0
    while i < len(argv):
        arg = argv[i]
        if arg == '--out-dir':
            replay.extend([arg, str(unit / 'objects')])
            i += 2
        elif arg.startswith('--emit='):
            replay.append('--emit=obj')
            i += 1
        else:
            replay.append(arg)
            i += 1
    replay.extend(['-Zcodegen-backend=' + str(args.backend)])
    env = dict(record['env'])
    env.update(supplements.get(str(capture), {}))
    env.update({'KFRC_GAP_SURVEY': str(report), 'KFRC_MACHINE': 'cranelift',
                'RUSTC_BOOTSTRAP': '1', 'KFRC_CODEGEN_THREADS': '1'})
    if args.cranelift_opt:
        env['KFRC_CRANELIFT_OPT'] = args.cranelift_opt
    receipt = {'capture': str(capture), 'argv': replay, 'cwd': record['cwd'],
               'env': env, 'backend_sha256': hashlib.sha256(args.backend.read_bytes()).hexdigest()}
    if str(capture) in supplements:
        receipt['env_supplement'] = {'path': str(args.env_supplement),
            'sha256': hashlib.sha256(args.env_supplement.read_bytes()).hexdigest(),
            'values': supplements[str(capture)]}
    (unit / 'invocation.json').write_text(json.dumps(receipt, indent=2) + '\n')
    started = time.time()
    print(f'SURVEY_START {args.project} {crate}', flush=True)
    with (unit / 'stdout.log').open('w') as stdout, (unit / 'stderr.log').open('w') as stderr:
        try:
            result = subprocess.run(replay, cwd=record['cwd'], env=env,
                                    stdout=stdout, stderr=stderr, timeout=args.unit_timeout)
            receipt['exit_code'] = result.returncode
        except subprocess.TimeoutExpired:
            receipt['exit_code'] = None
            receipt['timeout'] = True
    receipt['seconds'] = time.time() - started
    receipt['reports'] = []
    for path in sorted(report.glob('*.jsonl')):
        entries = [json.loads(line) for line in path.read_text().splitlines()]
        receipt['reports'].append({'path': str(path), 'sha256': hashlib.sha256(path.read_bytes()).hexdigest(),
                                   'rows': len(entries), 'summary': [r for r in entries if r.get('kind') == 'summary']})
    (unit / 'result.json').write_text(json.dumps(receipt, indent=2) + '\n')
    results.append({'crate': crate, 'unit': str(unit), 'exit_code': receipt['exit_code'],
                    'seconds': receipt['seconds'], 'reports': receipt['reports']})
    (out / 'index.json').write_text(json.dumps(results, indent=2) + '\n')
    print(f'SURVEY_COMPLETE {args.project} {crate} exit={receipt["exit_code"]} reports={len(receipt["reports"])} seconds={receipt["seconds"]:.1f}', flush=True)
    complete = bool(receipt['reports']) and all(
        any(s.get('status') == 'complete' for s in r['summary']) for r in receipt['reports'])
    if not complete:
        raise SystemExit(f'Incomplete survey for {crate}; pause and inspect {unit}/stderr.log')
print(f'PROJECT_SURVEY_COMPLETE {args.project} units={len(results)}', flush=True)
