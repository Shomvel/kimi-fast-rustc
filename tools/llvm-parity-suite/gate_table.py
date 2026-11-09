#!/usr/bin/env python3
"""gate_table.py <results-root> [--baseline llvm] [--out table.md]

Generates the kfrc/LLVM gate table from the per-arm JSON (<root>/<arm>/<crate>.json) and the
runtime JSON (<root>/runtime/<crate>.json). Every number in the output comes from those files.
Gates (docs/plan/poc/metrics.md): .text <= 1.0x LLVM, runtime <= 1.10x, compile instructions <= 1/3,
correctness = every test LLVM passes also passes, corpus output and workload checksums identical.
"""
import argparse
import json
import math
import pathlib
import statistics

p = argparse.ArgumentParser()
p.add_argument('root', type=pathlib.Path)
p.add_argument('--baseline', default='llvm')
p.add_argument('--out', type=pathlib.Path)
p.add_argument('--arms', help='comma-separated arm order (default: all, baseline first)')
p.add_argument('--label', action='append', default=[], help='arm=label shown in headers')
a = p.parse_args()
root = a.root
labels = dict(x.split('=', 1) for x in a.label)
suite = json.loads((pathlib.Path(__file__).resolve().parent / 'suite.json').read_text())
crates = [c['name'] for c in suite['crates']]
arm_dirs = sorted(d.name for d in root.iterdir() if d.is_dir() and any(d.glob('*.json')) and d.name != 'runtime')
arms = a.arms.split(',') if a.arms else [a.baseline] + [x for x in arm_dirs if x != a.baseline]
others = [x for x in arms if x != a.baseline]


def load(arm, crate):
    f = root / arm / f'{crate}.json'
    return json.loads(f.read_text()) if f.exists() else None


def load_rt(crate):
    f = root / 'runtime' / f'{crate}.json'
    return json.loads(f.read_text()) if f.exists() else None


def gmean(xs):
    xs = [x for x in xs if x]
    return math.exp(sum(math.log(x) for x in xs) / len(xs)) if xs else None


def fmt(x, digits=2):
    return '—' if x is None else f'{x:.{digits}f}'


def mark(x, limit):
    if x is None:
        return '—'
    return f'{x:.2f}' + (' ✓' if x <= limit else ' ✗')


def test_outcomes(rec, level):
    t = (rec or {}).get('tests', {}).get(level)
    if not t or 'binaries' not in t:
        return None
    out = {}
    for b, row in t['binaries'].items():
        for name, ev in row['outcomes'].items():
            out[(b.replace('-unit', ''), name)] = ev
    return out


def correctness(base, rec, crate):
    """Return a short status string and detail list for one arm vs the baseline."""
    if rec is None:
        return 'not run', []
    if rec['status'] != 'ok':
        return 'compile failed', rec.get('compile_error', [])[:2]
    notes, incomplete = [], []
    if not base or base.get('status') != 'ok':
        return 'incomplete: baseline unavailable', []
    for label, row in (('LLVM', base), ('candidate', rec)):
        driver = row.get('driver')
        if not driver or driver.get('exit_code') != 0:
            incomplete.append(f'{label} driver missing or build failed')
        corpus = row.get('corpus')
        if not corpus:
            incomplete.append(f'{label} corpus not run')
        elif corpus.get('exit_code') != 0 or not corpus.get('sha256') or not corpus.get('lines'):
            notes.append(f'corpus differs: {label} corpus failed or empty')
    if base.get('corpus') and rec.get('corpus'):
        if base['corpus'].get('sha256') != rec['corpus'].get('sha256'):
            notes.append('corpus differs from LLVM')
    expected = next(c for c in suite['crates'] if c['name'] == crate)['workloads']
    for workload, _ in expected:
        reference = base.get('workload_check', {}).get(workload)
        actual = rec.get('workload_check', {}).get(workload)
        if not reference or not actual:
            incomplete.append(f'{workload} checksum not run')
        elif (reference.get('exit_code') != 0 or actual.get('exit_code') != 0
              or reference.get('checksum') is None or actual.get('checksum') is None
              or reference['checksum'] != actual['checksum']):
            notes.append('checksum mismatch: ' + workload)
    if not base.get('tests') or not rec.get('tests'):
        incomplete.append('upstream tests not run')
    for level in ('itest', 'unit'):
        t = rec.get('tests', {}).get(level)
        bt = base.get('tests', {}).get(level)
        if not t and not bt:
            if level == 'unit' and next(c for c in suite['crates'] if c['name'] == crate)['unit_tests']:
                incomplete.append('unit tests not run')
            continue
        if not bt or 'binaries' not in bt:
            incomplete.append(f'{level} LLVM reference not built')
            continue
        if not t or 'binaries' not in t:
            incomplete.append(f'{level} missing or build failed: ' + '; '.join((t or {}).get('build_error', [])[:1]))
            continue
        bo, ro = test_outcomes(base, level) or {}, test_outcomes(rec, level) or {}
        passing = [k for k, v in bo.items() if v == 'ok']
        lost = [k for k in passing if ro.get(k) != 'ok']
        if lost:
            notes.append(f'{level}: {len(lost)} LLVM-passing tests not passing: ' + ', '.join(f'{b}::{n}={ro.get((b, n), "missing")}' for b, n in lost[:4]))
    if any(n.startswith(('corpus differs', 'checksum mismatch')) for n in notes):
        return 'FAIL: output', notes + incomplete
    if any('LLVM-passing tests not passing' in n for n in notes):
        return 'FAIL: tests', notes + incomplete
    if incomplete:
        return 'incomplete: missing validation', notes + incomplete
    return 'pass', notes


def tests_cell(rec, level):
    t = (rec or {}).get('tests', {}).get(level)
    if not t:
        return '—'
    if 'totals' not in t:
        return 'build fail'
    x = t['totals']
    s = f"{x['ok']}/{x['ok'] + x['failed'] + x['not_run']}"
    return s + (f" ({x['ignored']} ign)" if x['ignored'] else '')


lines = []
inputs = json.loads((root / 'inputs.json').read_text()) if (root / 'inputs.json').exists() else {}
lines.append(f'# kfrc 5-crate suite: gate table ({root.name})')
lines.append('')
lines.append('Generated by `gate_table.py` from `<arm>/<crate>.json` and `runtime/<crate>.json`; ratios are arm / LLVM.')
lines.append('Gates: `.text` ≤ 1.00, runtime ≤ 1.10, compile instructions ≤ 0.33 (≥ 3× fewer); correctness = corpus + checksums identical and every LLVM-passing upstream test passes.')
lines.append('Correctness: `FAIL: output` = corpus/checksum differs from LLVM (miscompile); `FAIL: tests` = an LLVM-passing upstream test fails; `incomplete` = a driver, corpus, checksum or required upstream test result is missing or could not be built. itest/unit = passed/total (itest harness compiled by LLVM, unit harness + library by the arm).')
lines.append('')
lines.append('| arm | backend sha256 |')
lines.append('|---|---|')
for arm in arms:
    rec = next((load(arm, c) for c in crates if load(arm, c)), None)
    lines.append(f"| {labels.get(arm, arm)} | `{(rec or {}).get('backend_sha256', '?')[:16]}` |")
lines.append('')
lines.append('Toolchain: ' + inputs.get('rustc', '?').splitlines()[0] if inputs.get('rustc') else 'Toolchain: ?')
lines.append(f"; host: {inputs.get('cpu')} × {inputs.get('nproc')}, kernel {inputs.get('kernel')}; sysroot manifest sha256 `{(inputs.get('sysroot_manifest_sha256') or '?')[:16]}`.")
lines.append('')
lines.append('Crates: ' + ', '.join(f"{c['name']} {c['version']} (`{c['sha256'][:12]}`)" for c in suite['crates']) + '.')
lines.append('')

summary = {}
for arm in others:
    lab = labels.get(arm, arm)
    lines.append(f'## {lab} vs LLVM')
    lines.append('')
    lines.append('| crate | correctness | itest | unit | .text | func syms | runtime (per workload, median of paired-round ratios [min–max]) | compile instr | instr − metadata | wall |')
    lines.append('|---|---|---|---|---:|---:|---|---:|---:|---:|')
    col = {'text': [], 'sym': [], 'rt': [], 'instr': [], 'cg': [], 'wall': []}
    details = []
    for crate in crates:
        base, rec, rt = load(a.baseline, crate), load(arm, crate), load_rt(crate)
        status, notes = correctness(base, rec, crate)
        details += [f'- {crate}: {n}' for n in notes]
        if rec is None or rec['status'] != 'ok' or base is None or base['status'] != 'ok':
            lines.append(f'| {crate} | {status} | — | — | — | — | — | — | — | — |')
            continue
        tr = rec['size']['text_bytes'] / base['size']['text_bytes']
        sr = rec['size']['func_symbol_bytes'] / base['size']['func_symbol_bytes']
        ir = rec['compile']['instructions_median'] / base['compile']['instructions_median']
        cg = (rec['compile'].get('codegen_instructions_estimate') or 0) / base['compile']['codegen_instructions_estimate'] if base['compile'].get('codegen_instructions_estimate') else None
        wr = rec['compile']['wall_median'] / base['compile']['wall_median']
        col['text'].append(tr); col['sym'].append(sr); col['instr'].append(ir); col['wall'].append(wr)
        if cg:
            col['cg'].append(cg)
        rtcells = []
        if rt:
            for w, wl in rt['workloads'].items():
                if arm not in wl['arms'] or a.baseline not in wl['arms']:
                    continue
                by_round = {}
                for s in wl['samples']:
                    if s['round'] >= 0 and s['exit_code'] == 0:
                        by_round.setdefault(s['round'], {})[s['arm']] = s['elapsed_ns']
                ratios = [v[arm] / v[a.baseline] for v in by_round.values() if arm in v and a.baseline in v]
                if not ratios:
                    rtcells.append(f'{w} fail')
                    continue
                r = statistics.median(ratios)
                same = wl['arms'][arm]['checksums'] == wl['arms'][a.baseline]['checksums']
                if same:
                    col['rt'].append(r)
                rtcells.append(f"{w} {r:.2f} [{min(ratios):.2f}–{max(ratios):.2f}]" + ('' if same else ' (checksum≠)') + (' invalid' if not same else (' ✓' if r <= 1.10 else ' ✗')))
        lines.append(f"| {crate} | {status} | {tests_cell(rec, 'itest')} | {tests_cell(rec, 'unit')} | {mark(tr, 1.0)} | {fmt(sr)} | {'<br>'.join(rtcells) or '—'} | {mark(ir, 1 / 3)} | {fmt(cg)} | {fmt(wr)} |")
    g = {k: gmean(v) for k, v in col.items()}
    summary[arm] = g
    lines.append(f"| **geomean** ({len(col['text'])} crates) | | | | {mark(g['text'], 1.0)} | {fmt(g['sym'])} | all {len(col['rt'])} workloads: {mark(g['rt'], 1.10)} | {mark(g['instr'], 1 / 3)} | {fmt(g['cg'])} | {fmt(g['wall'])} |")
    lines.append('')
    if details:
        lines.append('Correctness notes:')
        lines.append('')
        lines += details
        lines.append('')

lines.append('## Absolute values')
lines.append('')
lines.append('| crate | arm | status | .text B | func sym B | rodata B | compile instr (G) | metadata-only instr (G) | wall s | maxRSS MiB | itest | unit | corpus sha |')
lines.append('|---|---|---|---:|---:|---:|---:|---:|---:|---:|---|---|---|')
for crate in crates:
    for arm in arms:
        rec = load(arm, crate)
        if rec is None:
            continue
        lab = labels.get(arm, arm)
        if rec['status'] != 'ok':
            lines.append(f"| {crate} | {lab} | compile failed: {(rec.get('compile_error') or ['?'])[0][:140].replace('|', '/')} | | | | | | | | | | |")
            continue
        s, cpl = rec['size'], rec['compile']
        lines.append(f"| {crate} | {lab} | ok | {s['text_bytes']} | {s['func_symbol_bytes']} | {s['rodata_bytes']} | "
                     f"{cpl['instructions_median'] / 1e9:.3f} | {cpl['metadata_instructions_median'] / 1e9:.3f} | {cpl['wall_median']:.2f} | "
                     f"{cpl['maxrss_kib_median'] / 1024:.0f} | {tests_cell(rec, 'itest')} | {tests_cell(rec, 'unit')} | `{rec.get('corpus', {}).get('sha256', '?')[:12]}` |")
lines.append('')
lines.append('## Runtime medians (ms, [min–max], 11 interleaved rounds after one warm-up)')
lines.append('')
lines.append('| crate | workload | iterations | ' + ' | '.join(labels.get(x, x) for x in arms) + ' |')
lines.append('|---|---|---:|' + '---|' * len(arms))
for crate in crates:
    rt = load_rt(crate)
    if not rt:
        continue
    for w, wl in rt['workloads'].items():
        cells = []
        for arm in arms:
            x = wl['arms'].get(arm)
            if not x or x['median_ns'] is None:
                cells.append('—')
            else:
                cells.append(f"{x['median_ns'] / 1e6:.3f} [{x['min_ns'] / 1e6:.3f}–{x['max_ns'] / 1e6:.3f}]")
        lines.append(f"| {crate} | {w} | {wl['iterations']} | " + ' | '.join(cells) + ' |')
lines.append('')
lines.append('## Driver objects (LLVM-compiled; identical across arms means timing differences come from the library only)')
lines.append('')
lines.append('| crate | driver-local .text B | objects identical to LLVM arm |')
lines.append('|---|---:|---|')
for crate in crates:
    base = load(a.baseline, crate)
    if not base or 'driver' not in base or 'objects_sha256' not in base['driver']:
        continue
    same = []
    for arm in others:
        rec = load(arm, crate)
        if rec and rec.get('driver', {}).get('objects_sha256'):
            same.append(f"{labels.get(arm, arm)}: {'yes' if rec['driver']['objects_sha256'] == base['driver']['objects_sha256'] else 'no'}")
    lines.append(f"| {crate} | {base['driver']['local_text_bytes']} | {', '.join(same) or '—'} |")
lines.append('')

text = '\n'.join(lines) + '\n'
out = a.out or (root / 'table.md')
out.write_text(text)
(out.with_suffix('.json')).write_text(json.dumps(summary, indent=2) + '\n')
print(text)
