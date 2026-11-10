"""Recompute the LLVM gap from the accepted final1 evidence; no compilation."""
from pathlib import Path
import argparse
import collections
import hashlib
import json
import math
import re
import statistics

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('evidence', type=Path, help='Unpacked final1 evidence root')
parser.add_argument('--output', type=Path, required=True)
args = parser.parse_args()
ROOT = args.evidence.resolve()
OUT = args.output.resolve()
OUT.mkdir(parents=True, exist_ok=True)
RUN = ROOT / 'integrated-final1'
CRATES = ['semver', 'base64', 'unicode-segmentation', 'humantime']
inputs = {}

def read(path):
    inputs[str(path.relative_to(ROOT))] = hashlib.sha256(path.read_bytes()).hexdigest()
    return path.read_text()

def js(path):
    return json.loads(read(path))

def geo(values):
    return math.exp(statistics.mean(math.log(x) for x in values))

def perf(path):
    result = {}
    for line in read(path).splitlines():
        fields = line.split(',')
        if len(fields) >= 5 and fields[0].isdigit():
            assert float(fields[4]) == 100.0
            result[fields[2]] = int(fields[0])
    return result

def symbols(path):
    result = collections.Counter()
    for line in read(path).splitlines():
        fields = line.split()
        if len(fields) == 4 and fields[2] in 'tTwW':
            # suite.py explicitly invokes nm -t d: sizes are DECIMAL.
            result[re.sub(r'\.llvm\.\d+$', '', fields[3])] += int(fields[1])
    return result

summary = js(ROOT / 'summary.json')
rows = []
runtime = []
for crate in CRATES:
    arms = {a: js(RUN / f'suite/{a}/{crate}.json') for a in ['llvm', 'integrated']}
    k, l = arms['integrated'], arms['llvm']
    stages = collections.Counter()
    functions = collections.Counter()
    x86_functions = collections.Counter()
    for line in read(RUN / f'profile/integrated/{crate}/stages-check0/stages.jsonl').splitlines():
        d = json.loads(line)
        assert d['status'] == 'ok' and d.get('instructions') is not None
        stages[d['stage']] += d['instructions']
        functions[d['symbol']] += d['instructions']
        if d['stage'] == 'x86':
            x86_functions[d['symbol']] += d['instructions']
    syms = {a: symbols(RUN / f'suite/{a}/{crate}/ref/symbol-sizes.txt') for a in arms}
    for a in arms:
        assert sum(syms[a].values()) == arms[a]['size']['func_symbol_bytes']
    ks, ls = syms['integrated'], syms['llvm']
    shared = set(ks) & set(ls)
    cost = next(x for x in summary['rows'] if x['crate'] == crate)
    totals = {}
    for arm, folder, filename, key in [
        ('integrated', 'total-check0', 'total.jsonl', 'window_final'),
        ('llvm', 'llvm-total', 'llvm.jsonl', 'window_llvm'),
    ]:
        entries = read(RUN / f'profile/integrated/{crate}/{folder}/{filename}').splitlines()
        assert len(entries) == 1
        total = json.loads(entries[0])
        assert total['scope'] == 'initial_process_threads_and_descendants_user_instructions'
        assert total['status'] == 'ok' and total['instructions'] == cost[key]
        assert all(root['time_enabled_ns'] == root['time_running_ns'] for root in total['roots'])
        totals[arm] = total
    row = {'crate': crate, 'arms': {a: {'size': d['size'], 'compile': {
        key: value for key, value in d['compile'].items() if 'median' in key
    }} for a, d in arms.items()}, 'stages': dict(stages.most_common()),
        'stage_sum': sum(stages.values()), 'top_compile_functions': functions.most_common(8),
        'top_x86_functions': x86_functions.most_common(5),
        'symbols': {'common_count': len(shared),
            'common_kfrc_bytes': sum(ks[n] for n in shared),
            'common_llvm_bytes': sum(ls[n] for n in shared),
            'kfrc_only_bytes': sum(ks[n] for n in ks.keys() - ls.keys()),
            'llvm_only_bytes': sum(ls[n] for n in ls.keys() - ks.keys()),
            'top_kfrc': ks.most_common(8),
            'top_excess': sorted([(ks[n] - ls.get(n, 0), ks[n], ls.get(n, 0), n)
                                  for n in ks], reverse=True)[:8]},
        'exact_codegen_kfrc': cost['window_final'], 'exact_codegen_llvm': cost['window_llvm'],
        'codegen_windows': totals}
    row['ratios'] = {key: k['size'][key] / l['size'][key] for key in k['size']}
    row['ratios'].update({key: k['compile'][key] / l['compile'][key]
                         for key in ['instructions_median', 'wall_median', 'maxrss_kib_median']})
    row['ratios']['exact_codegen'] = cost['window_final'] / cost['window_llvm']
    rows.append(row)
    rt = js(RUN / f'suite/runtime/{crate}.json')
    for name, data in rt['workloads'].items():
        samples = {a: {s['round']: s['elapsed_ns'] for s in data['samples']
                      if s['arm'] == a and s['round'] >= 0} for a in arms}
        assert all(len(v) == 11 for v in samples.values())
        checksums = {s['checksum'] for s in data['samples'] if s['arm'] in arms}
        assert len(checksums) == 1
        pairs = [samples['integrated'][i] / samples['llvm'][i] for i in samples['llvm']]
        p = {a: perf(RUN / f'suite/runtime/{crate}/{name}-{a}.perf.csv') for a in arms}
        runtime.append({'crate': crate, 'workload': name, 'iterations': data['iterations'],
            'llvm_ms': statistics.median(samples['llvm'].values()) / 1e6,
            'kfrc_ms': statistics.median(samples['integrated'].values()) / 1e6,
            'paired_ratio': statistics.median(pairs), 'paired_min': min(pairs), 'paired_max': max(pairs),
            'instructions_ratio': p['integrated']['instructions:u'] / p['llvm']['instructions:u'],
            'cycles_ratio': p['integrated']['cycles:u'] / p['llvm']['cycles:u'],
            'perf': p})

result = {'scope': 'accepted final1; reanalysis, no new benchmark runs',
          'rows': rows, 'runtime': runtime,
          'geomeans': {key: geo([r['ratios'][key] for r in rows]) for key in rows[0]['ratios']}}
result['geomeans']['runtime_paired'] = geo([r['paired_ratio'] for r in runtime])
assert abs(result['geomeans']['runtime_paired'] - summary['llvm_geomean_gates']['rt']) < 1e-12
(OUT / 'metrics.json').write_text(json.dumps(result, indent=2) + '\n')
(OUT / 'input-hashes.json').write_text(json.dumps(inputs, indent=2) + '\n')

lines = ['# KFRC 与 LLVM 差距数据表', '',
         '从最终验收记录重新计算；所有比值为 KFRC / LLVM。', '',
         '| crate | .text B：LLVM → KFRC | .text 比值 | 整进程编译指令比值 | 精确 codegen 指令比值 | 编译墙钟 s：LLVM → KFRC | 峰值 RSS MiB：LLVM → KFRC |',
         '|---|---:|---:|---:|---:|---:|---:|']
for r in rows:
    l, k = r['arms']['llvm'], r['arms']['integrated']
    lines.append(f"| {r['crate']} | {l['size']['text_bytes']:,} → {k['size']['text_bytes']:,} | {r['ratios']['text_bytes']:.3f} | {r['ratios']['instructions_median']:.3f} | {r['ratios']['exact_codegen']:.3f} | {l['compile']['wall_median']:.3f} → {k['compile']['wall_median']:.3f} | {l['compile']['maxrss_kib_median']/1024:.1f} → {k['compile']['maxrss_kib_median']/1024:.1f} |")
lines += ['', '| crate / 负载 | 迭代次数 | LLVM ms | KFRC ms | 配对耗时比值 | 指令比值 | cycles 比值 |',
          '|---|---:|---:|---:|---:|---:|---:|']
for r in runtime:
    lines.append(f"| {r['crate']} / {r['workload']} | {r['iterations']:,} | {r['llvm_ms']:.3f} | {r['kfrc_ms']:.3f} | {r['paired_ratio']:.3f} | {r['instructions_ratio']:.3f} | {r['cycles_ratio']:.3f} |")
lines += ['', '墙钟为 11 轮交错计时；配对比值取逐轮比值中位数。硬件计数为另行采集的单次全进程 profile，不与循环内计时混为同一统计量。', '',
          '| crate | .rodata + .data.rel.ro B：LLVM → KFRC | .o 总文件 B：LLVM → KFRC | rlib 文件 B：LLVM → KFRC | 函数符号数：LLVM → KFRC |',
          '|---|---:|---:|---:|---:|']
for r in rows:
    l, k = r['arms']['llvm']['size'], r['arms']['integrated']['size']
    lines.append('| ' + r['crate'] + ' | ' + ' | '.join(f"{l[key]:,} → {k[key]:,}" for key in ['rodata_bytes','object_bytes','rlib_bytes','func_symbols']) + ' |')
lines += ['', '| 编译阶段占已计数阶段比例 | semver | base64 | unicode-segmentation | humantime |', '|---|---:|---:|---:|---:|']
groups = {'x86 降低与分配': ['x86'], '内联后优化': ['post_inline_optimize_s'], '内联': ['inline_s'], '布局降低': ['layout'], '初次优化': ['optimize_s'], '显式验证阶段': ['verify_l','verify_s_input','verify_s_output']}
for label, keys in groups.items():
    lines.append('| ' + label + ' | ' + ' | '.join(f"{sum(r['stages'].get(k,0) for k in keys)/r['stage_sum']:.1%}" for r in rows) + ' |')
lines += ['', '阶段插桩使用 current-thread user instructions；其总和不覆盖精确 codegen 窗口的全部工作。', '']
(OUT / 'tables.md').write_text('\n'.join(lines))
print(json.dumps({'geomeans':result['geomeans'], 'inputs':len(inputs), 'runtime_rows':len(runtime)}, indent=2))
