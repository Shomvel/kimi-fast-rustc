#!/usr/bin/env python3
"""Per-stage instruction tables from semver_arms.py result directories.

usage: python3 -I stage_compare.py [--stages-arm stages-check1] [--total-arm total-check1]
           [--llvm DIR] LABEL=DIR [LABEL=DIR ...]
Prints one markdown table: per-stage sums (KFRC_PROFILE windows), sum of windows,
the separate whole-backend total window (KFRC_PROFILE_TOTAL) and object identity.
The first LABEL is the base for the relative columns.
"""
import argparse
import collections
import json
from pathlib import Path

ORDER = ["prepare_r", "specialize_r", "build", "verify_s_input", "optimize_s", "identity_s",
         "inline_s", "post_inline_identity_s", "post_inline_optimize_s", "verify_s_output",
         "layout", "verify_l", "x86", "object"]
parser = argparse.ArgumentParser()
parser.add_argument('--stages-arm', default='stages-check1')
parser.add_argument('--total-arm', default='total-check1')
parser.add_argument('--llvm', type=Path)
parser.add_argument('--lowered-split', action='store_true')
parser.add_argument('runs', nargs='+')
args = parser.parse_args()


def records(path):
    rows = [json.loads(line) for line in path.read_text().splitlines()]
    assert all(row['status'] == 'ok' for row in rows), path
    return rows


def arm(folder, name):
    return json.loads((folder / name / 'arm.json').read_text())


runs = []
for item in args.runs:
    label, folder = item.split('=', 1)
    folder = Path(folder)
    rows = records(folder / args.stages_arm / 'stages.jsonl')
    per = collections.Counter()
    count = collections.Counter()
    for row in rows:
        per[row['stage']] += row['instructions']
        count[row['stage']] += 1
    total = arm(folder, args.total_arm)
    stages = arm(folder, args.stages_arm)
    runs.append({'label': label, 'per': per, 'count': count, 'rows': rows,
                 'total': total['total_instructions'],
                 'identical': total['objects_identical_to_reference'] and stages['objects_identical_to_reference']})
base = runs[0]


def cell(value, reference, first):
    if first or not reference:
        return f'{value:,}'
    return f'{value:,} ({(value - reference) / reference:+.1%})'


labels = [run['label'] for run in runs]
print(f'stage windows: `{args.stages_arm}`; total window: `{args.total_arm}`')
print()
print('| stage | windows | ' + ' | '.join(labels) + ' |')
print('|---|---:|' + '---:|' * len(runs))
for stage in ORDER + sorted(set().union(*(run['per'] for run in runs)) - set(ORDER)):
    if not any(run['per'][stage] for run in runs):
        continue
    windows = '/'.join(sorted({str(run['count'][stage]) for run in runs}))
    cells = [cell(run['per'][stage], base['per'][stage], index == 0) for index, run in enumerate(runs)]
    print(f'| {stage} | {windows} | ' + ' | '.join(cells) + ' |')
sums = [sum(run['per'].values()) for run in runs]
print('| sum of stage windows | | ' + ' | '.join(cell(value, sums[0], index == 0) for index, value in enumerate(sums)) + ' |')
print('| outside stage windows (total − sum) | | ' + ' | '.join(
    f'{run["total"] - value:,}' for run, value in zip(runs, sums)) + ' |')
print('| **total window** | | ' + ' | '.join(
    cell(run['total'], base['total'], index == 0) for index, run in enumerate(runs)) + ' |')
if args.llvm:
    llvm = arm(args.llvm, 'llvm-total')['total_instructions']
    print('| total ÷ LLVM total (' + f'{llvm:,}' + ') | | ' + ' | '.join(
        f'{run["total"] / llvm:.3f}×' for run in runs) + ' |')
print('| objects identical to reference | | ' + ' | '.join(
    'yes' if run['identical'] else 'NO' for run in runs) + ' |')
if args.lowered_split:
    print()
    print('| run | instances with S graph | lowered to x86 | S work of never-lowered instances | share of total |')
    print('|---|---:|---:|---:|---:|')
    for run in runs:
        lowered = {row['symbol'] for row in run['rows'] if row['stage'] == 'x86'}
        s_stages = {'specialize_r', 'build', 'verify_s_input', 'optimize_s', 'prepare_r'}
        instances = {row['symbol'] for row in run['rows'] if row['stage'] in ('build', 'specialize_r', 'optimize_s')}
        unlowered = sum(row['instructions'] for row in run['rows']
                        if row['stage'] in s_stages - {'prepare_r'} and row['symbol'] not in lowered)
        print(f'| {run["label"]} | {len(instances)} | {len(lowered)} | {unlowered:,} | {unlowered / run["total"]:.2%} |')
