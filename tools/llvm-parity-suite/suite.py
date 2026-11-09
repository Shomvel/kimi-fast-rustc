#!/usr/bin/env python3
"""kfrc 5-crate suite runner (run with python3 -I inside the E2B sandbox).

Subcommands:
  prepare <root>                       verify vendored .crate hashes, extract to <root>/src, record inputs
  arm <root> <arm-name> <backend|llvm> library build, size, compile cost, LLVM driver, corpus, upstream tests
  runtime <root> <arm> [<arm> ...]     serial interleaved driver timing over arms, then one perf-stat profile
  calibrate <root> <arm>               one run of every workload (for tuning iteration counts)

Each arm writes <root>/<arm>/<crate>.json; runtime writes <root>/runtime/<crate>.json.
No compiler sources are touched; the backend is only passed as -Zcodegen-backend.
"""
import hashlib
import json
import os
import pathlib
import re
import shutil
import statistics
import subprocess
import sys
import tarfile
import time
import tomllib

SUITE = pathlib.Path(__file__).resolve().parent
CONFIG = json.loads((SUITE / 'suite.json').read_text())
TC = '+' + CONFIG['toolchain']
TARGET = CONFIG['target']
PERF = CONFIG['perf']


def sha(path):
    h = hashlib.sha256()
    with open(path, 'rb') as f:
        for block in iter(lambda: f.read(1 << 20), b''):
            h.update(block)
    return h.hexdigest()


def tree_hashes(root):
    root = pathlib.Path(root)
    return {str(p.relative_to(root)): sha(p) for p in sorted(root.rglob('*')) if p.is_file()}


def clean_env():
    env = {k: v for k, v in os.environ.items()
           if not k.startswith('KFRC_') and k not in ('RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS', 'CARGO_TARGET_DIR')}
    env['RUSTC_BOOTSTRAP'] = '1'
    return env


def write_json(path, value):
    path.write_text(json.dumps(value, indent=2) + '\n')


def run_logged(cmd, stem, env=None, cwd=None, timeout=3600):
    """Run cmd with stdout/stderr to <stem>.stdout/.stderr; return (exit, seconds, rusage)."""
    stem.parent.mkdir(parents=True, exist_ok=True)
    start = time.perf_counter()
    with open(str(stem) + '.stdout', 'w') as o, open(str(stem) + '.stderr', 'w') as e:
        proc = subprocess.Popen(cmd, stdout=o, stderr=e, env=env or clean_env(), cwd=cwd)
        try:
            _, status, usage = wait4_timeout(proc, timeout)
            code = os.waitstatus_to_exitcode(status)
        except subprocess.TimeoutExpired:
            proc.kill()
            proc.wait()
            code, usage = 'timeout', None
    seconds = time.perf_counter() - start
    write_json(pathlib.Path(str(stem) + '.command.json'), {'command': [str(c) for c in cmd], 'cwd': str(cwd) if cwd else None,
                                                           'exit_code': code, 'wall_seconds': seconds})
    return code, seconds, usage


def wait4_timeout(proc, timeout):
    deadline = time.monotonic() + timeout
    while True:
        pid, status, usage = os.wait4(proc.pid, os.WNOHANG)
        if pid:
            proc.returncode = os.waitstatus_to_exitcode(status)
            return pid, status, usage
        if time.monotonic() > deadline:
            raise subprocess.TimeoutExpired(proc.args, timeout)
        time.sleep(0.005)


def crate_dir(root, c):
    return pathlib.Path(root) / 'src' / f"{c['name']}-{c['version']}"


def features_args(c):
    out = []
    for f in c['features']:
        out += ['--cfg', f'feature="{f}"']
    return out


def library_cmd(root, c, backend, output, emit=None):
    src = crate_dir(root, c)
    manifest = tomllib.loads((src / 'Cargo.toml').read_text())
    lib_path = manifest.get('lib', {}).get('path', 'src/lib.rs')
    cmd = ['rustc', TC, '--sysroot=' + CONFIG['sysroot'], '--crate-name', c['lib'], '--crate-type', 'rlib',
           f"--edition={c['edition']}", str(src / lib_path)] + features_args(c) + CONFIG['library_flags'] + ['--cap-lints=allow']
    if emit:
        cmd.append('--emit=' + emit)
    cmd += ['-o', str(output)]
    if backend != 'llvm':
        cmd.append('-Zcodegen-backend=' + backend)
    return cmd


# ---------------------------------------------------------------- prepare
def prepare(root):
    root = pathlib.Path(root)
    (root / 'src').mkdir(parents=True, exist_ok=True)
    record = {'crates': {}, 'suite_files': {}}
    for c in CONFIG['crates']:
        archive = SUITE / 'crates' / f"{c['name']}-{c['version']}" / f"{c['name']}-{c['version']}.crate"
        digest = sha(archive)
        if digest != c['sha256']:
            raise SystemExit(f"archive hash mismatch {archive}: {digest}")
        dest = crate_dir(root, c)
        if not dest.exists():
            with tarfile.open(archive) as t:
                for m in t.getmembers():
                    p = pathlib.PurePosixPath(m.name)
                    if p.is_absolute() or '..' in p.parts or p.parts[0] != dest.name or not (m.isfile() or m.isdir()):
                        raise SystemExit(f'bad archive entry {m.name}')
                t.extractall(root / 'src', filter='data')
        record['crates'][c['name']] = {'version': c['version'], 'archive_sha256': digest, 'source_files_sha256': tree_hashes(dest)}
    for p in sorted(SUITE.rglob('*')):
        if p.is_file() and p.suffix in ('.py', '.rs', '.sh', '.json', '.lock') and 'results' not in p.parts:
            record['suite_files'][str(p.relative_to(SUITE))] = sha(p)
    record['rustc'] = subprocess.run(['rustc', TC, '-vV'], capture_output=True, text=True).stdout
    record['cargo'] = subprocess.run(['cargo', TC, '-V'], capture_output=True, text=True).stdout
    record['sysroot'] = CONFIG['sysroot']
    manifest = pathlib.Path(CONFIG['sysroot']).parent / 'manifest.json'
    record['sysroot_manifest_sha256'] = sha(manifest) if manifest.exists() else None
    record['kernel'] = os.uname().release
    record['nproc'] = os.cpu_count()
    record['cpu'] = next((l.split(':', 1)[1].strip() for l in open('/proc/cpuinfo') if l.startswith('model name')), None)
    write_json(root / 'inputs.json', record)
    print(json.dumps({'prepared': list(record['crates'])}))


def verify_sources(root, c):
    record = json.loads((pathlib.Path(root) / 'inputs.json').read_text())['crates'][c['name']]
    if tree_hashes(crate_dir(root, c)) != record['source_files_sha256']:
        raise SystemExit(f"source tree of {c['name']} changed since prepare")


# ---------------------------------------------------------------- size
def object_sizes(objects):
    objs = sorted(str(p) for p in objects)
    sizes = subprocess.run(['size', '-A', '-d', *objs], capture_output=True, text=True, check=True).stdout
    text = rodata = 0
    for line in sizes.splitlines():
        parts = line.split()
        if len(parts) >= 2 and parts[1].isdigit():
            if parts[0] == '.text' or parts[0].startswith('.text.'):
                text += int(parts[1])
            elif parts[0].startswith('.rodata') or parts[0].startswith('.data.rel.ro'):
                rodata += int(parts[1])
    syms = subprocess.run(['readelf', '-Ws', '--wide', *objs], capture_output=True, text=True, check=True).stdout
    func_bytes = func_count = 0
    for line in syms.splitlines():
        parts = line.split()
        if len(parts) >= 8 and parts[3] == 'FUNC' and parts[6] != 'UND':
            func_bytes += int(parts[2], 0)
            func_count += 1
    return {'text_bytes': text, 'func_symbol_bytes': func_bytes, 'func_symbols': func_count, 'rodata_bytes': rodata,
            'object_count': len(objs), 'object_bytes': sum(os.path.getsize(p) for p in objs)}, sizes


def unpack(rlib, dest):
    dest.mkdir(parents=True, exist_ok=True)
    subprocess.run(['ar', 'x', str(rlib)], cwd=dest, check=True)
    return sorted(dest.glob('*.o'))


# ---------------------------------------------------------------- compile cost
def perf_compile(cmd, folder):
    folder.mkdir(parents=True, exist_ok=True)
    stat = folder / 'perf-stat.csv'
    code, seconds, usage = run_logged([PERF, 'stat', '-x,', '-e', 'instructions:u,cycles:u', '-o', str(stat), '--'] + cmd,
                                      folder / 'build')
    row = {'exit_code': code, 'wall_seconds': seconds}
    if usage:
        row.update(user_seconds=usage.ru_utime, sys_seconds=usage.ru_stime, maxrss_kib=usage.ru_maxrss)
    if code == 0:
        for line in stat.read_text().splitlines():
            parts = line.split(',')
            if len(parts) > 4 and parts[0].strip().isdigit():
                row[parts[2].split(':')[0]] = int(parts[0])
                row[parts[2].split(':')[0] + '_running_pct'] = float(parts[4]) if parts[4] else None
    return row


def med(rows, key):
    vals = [r[key] for r in rows if r.get(key) is not None]
    return statistics.median(vals) if vals else None


# ---------------------------------------------------------------- tests
def toml_value(v):
    if isinstance(v, bool):
        return 'true' if v else 'false'
    if isinstance(v, str):
        return json.dumps(v)
    if isinstance(v, list):
        return '[' + ', '.join(toml_value(x) for x in v) + ']'
    if isinstance(v, dict):
        return '{ ' + ', '.join(f'{json.dumps(k)} = {toml_value(x)}' for k, x in v.items()) + ' }'
    raise TypeError(v)


def write_wrapper(root, c, backend, wdir):
    src = crate_dir(root, c)
    upstream = tomllib.loads((src / 'Cargo.toml').read_text())
    shutil.copytree(src, wdir / 'upstream')
    (wdir / 'src').mkdir(parents=True)
    (wdir / 'src' / 'lib.rs').write_text('')
    if (src / 'tests').exists():
        shutil.copytree(src / 'tests', wdir / 'tests')
    dev = upstream.get('dev-dependencies', {})
    lines = ['cargo-features = ["codegen-backend"]', '[package]', f'name = "suite-itest-{c["name"]}"', 'version = "0.0.0"',
             'edition = "2021"', 'autotests = false', 'publish = false', '',
             '[workspace]', 'members = ["upstream"]', 'resolver = "2"', '',
             '[dependencies]',
             f'{c["name"]} = {toml_value({"path": "upstream", "default-features": False, "features": c["features"]})}', '',
             '[dev-dependencies]']
    for name in c['itest_dev_deps']:
        spec = dev[name]
        lines.append(f'{name} = {toml_value(spec if isinstance(spec, dict) else {"version": spec})}')
    for t in upstream.get('test', []):
        lines += ['', '[[test]]', f'name = {json.dumps(t["name"])}', f'path = {json.dumps(t.get("path", "tests/" + t["name"] + ".rs"))}',
                  f'edition = {json.dumps(upstream["package"].get("edition", "2015"))}']
    lines += ['', '[profile.release]', 'opt-level = 3', 'codegen-units = 16', 'panic = "abort"', 'debug = 0',
              'overflow-checks = false', 'debug-assertions = false', 'incremental = false', 'lto = false']
    if backend != 'llvm':
        lines += ['', f'[profile.release.package.{c["name"]}]', f'codegen-backend = {json.dumps(backend)}']
    (wdir / 'Cargo.toml').write_text('\n'.join(lines) + '\n')
    lock = SUITE / 'crates' / f"{c['name']}-{c['version']}" / 'tests.Cargo.lock'
    if lock.exists():
        shutil.copy(lock, wdir / 'Cargo.lock')
        return True
    return False


def run_test_binary(exe, cwd, folder):
    folder.mkdir(parents=True, exist_ok=True)
    env = clean_env()
    code, _, _ = run_logged([exe, '--list'], folder / 'list', env=env, cwd=cwd, timeout=300)
    listed = [l[:-len(': test')] for l in (folder / 'list.stdout').read_text().splitlines() if l.endswith(': test')]
    code, seconds, _ = run_logged([exe, '--test-threads=1', '--include-ignored', '-Zunstable-options', '--format=json'],
                                  folder / 'run', env=env, cwd=cwd, timeout=1200)
    outcomes = {}
    for line in (folder / 'run.stdout').read_text().splitlines():
        try:
            ev = json.loads(line)
        except json.JSONDecodeError:
            continue
        if ev.get('type') == 'test' and ev.get('event') in ('ok', 'failed', 'ignored', 'timeout'):
            outcomes[ev['name']] = ev['event']
    for name in listed:
        outcomes.setdefault(name, 'not_run')
    counts = {k: sum(1 for v in outcomes.values() if v == k) for k in ('ok', 'failed', 'ignored', 'not_run')}
    return {'exit_code': code, 'seconds': seconds, 'listed': len(listed), 'counts': counts, 'outcomes': outcomes}


def json_errors(stdout_path):
    """Rendered compiler errors from cargo --message-format=json (diagnostics go to stdout)."""
    errors = []
    for line in pathlib.Path(stdout_path).read_text().splitlines():
        try:
            msg = json.loads(line)
        except json.JSONDecodeError:
            continue
        if msg.get('reason') == 'compiler-message' and msg['message']['level'] == 'error':
            errors.append(msg['message']['rendered'].strip().splitlines()[0][:400])
    return errors[:8]


def cargo_tests(root, c, backend, out, cdir):
    """Upstream tests: itest = upstream tests/*.rs compiled by LLVM against the arm's library;
    unit = the crate's own #[cfg(test)] build (library + harness compiled by the arm)."""
    wdir = cdir / 'tests-workspace'
    locked = write_wrapper(root, c, backend, wdir)
    env = clean_env()
    env['CARGO_TARGET_DIR'] = str(out / 'cargo-target')
    env['CARGO_ENCODED_RUSTFLAGS'] = '-Zmir-opt-level=0'
    base = ['cargo', TC, 'test', '--release', '--target', TARGET, '-Zbuild-std=std,panic_abort', '-Zpanic-abort-tests',
            '--no-run', '--message-format=json', '-v']
    if locked:
        base.append('--locked')
    else:
        code, _, _ = run_logged(['cargo', TC, 'generate-lockfile'], cdir / 'tests' / 'generate-lockfile', env=env, cwd=wdir, timeout=600)
        if code == 0:
            shutil.copy(wdir / 'Cargo.lock', cdir / 'tests.Cargo.lock')
    result = {'lockfile': 'suite' if locked else 'generated', 'sysroot': 'cargo -Zbuild-std (MIR opt-level 0, panic=abort, O3, CGU16)'}
    levels = []
    if (wdir / 'tests').exists() and tomllib.loads((wdir / 'Cargo.toml').read_text()).get('test'):
        levels.append(('itest', ['-p', f'suite-itest-{c["name"]}', '--tests']))
    if c['unit_tests']:
        feats = ['--no-default-features'] + (['--features', ','.join(c['features'])] if c['features'] else [])
        levels.append(('unit', ['-p', c['name'], '--lib'] + feats))
    for level, args in levels:
        folder = cdir / 'tests' / level
        code, seconds, _ = run_logged(base + args, folder / 'build', env=env, cwd=wdir, timeout=3600)
        log = (folder / 'build.stderr').read_text()
        invocations = [l for l in log.splitlines() if 'Running `' in l and f'--crate-name {c["lib"]} ' in l]
        row = {'build_exit_code': code, 'build_seconds': seconds,
               'crate_invocations': len(invocations),
               'crate_invocations_with_backend': sum(1 for l in invocations if 'codegen-backend' in l)}
        if code != 0:
            row['build_error'] = json_errors(folder / 'build.stdout') or [l for l in log.splitlines() if l.startswith('error')][:8]
            result[level] = row
            continue
        binaries = {}
        for line in (folder / 'build.stdout').read_text().splitlines():
            try:
                msg = json.loads(line)
            except json.JSONDecodeError:
                continue
            if msg.get('reason') == 'compiler-artifact' and msg.get('executable') and msg['profile'].get('test'):
                pkg_dir = pathlib.Path(msg['manifest_path']).parent
                binaries[msg['target']['name'] + ('-unit' if level == 'unit' else '')] = (msg['executable'], pkg_dir)
        row['binaries'] = {}
        for name, (exe, pkg_dir) in sorted(binaries.items()):
            row['binaries'][name] = run_test_binary(exe, pkg_dir, folder / name)
        totals = {k: sum(b['counts'][k] for b in row['binaries'].values()) for k in ('ok', 'failed', 'ignored', 'not_run')}
        row['totals'] = totals
        row['all_exit_zero'] = all(b['exit_code'] == 0 for b in row['binaries'].values())
        result[level] = row
    return result


# ---------------------------------------------------------------- arm
def arm(root, arm_name, backend):
    root = pathlib.Path(root)
    out = root / arm_name
    out.mkdir(parents=True, exist_ok=True)  # per-crate dirs below refuse reuse
    backend_sha = 'LLVM' if backend == 'llvm' else sha(backend)
    rounds = int(os.environ.get('SUITE_COMPILE_ROUNDS', '3'))
    only = os.environ.get('SUITE_CRATES')
    for c in CONFIG['crates']:
        if only and c['name'] not in only.split(','):
            continue
        verify_sources(root, c)
        cdir = out / c['name']
        cdir.mkdir()
        rec = {'crate': c['name'], 'version': c['version'], 'arm': arm_name, 'backend': backend, 'backend_sha256': backend_sha,
               'flags': CONFIG['library_flags'], 'sysroot': CONFIG['sysroot']}
        print(f"[{arm_name}] {c['name']}: reference build", flush=True)
        ref = cdir / 'ref'
        cmd = library_cmd(root, c, backend, ref / 'lib.rlib')
        code, seconds, _ = run_logged(cmd, ref / 'build', timeout=3600)
        rec['library_command'] = cmd
        if code != 0:
            err = (ref / 'build.stderr').read_text().splitlines()
            rec['status'] = 'compile_failed'
            rec['compile_error'] = [l for l in err if 'error' in l][:6] or err[-6:]
            write_json(out / f"{c['name']}.json", rec)
            print(f"[{arm_name}] {c['name']}: compile failed: {rec['compile_error'][:1]}", flush=True)
            continue
        rec['status'] = 'ok'
        objects = unpack(ref / 'lib.rlib', ref / 'objects')
        rec['size'], sections = object_sizes(objects)
        (ref / 'size-sections.txt').write_text(sections)
        rec['size']['rlib_bytes'] = (ref / 'lib.rlib').stat().st_size
        ref_hashes = {p.name: sha(p) for p in objects}
        symbols = subprocess.run(['readelf', '-Ws', '--wide', *map(str, objects)], capture_output=True, text=True).stdout
        rec['kfrc_file_symbol'] = 'FILE' in symbols and ' kfrc' in symbols
        with open(ref / 'symbol-sizes.txt', 'w') as f:
            subprocess.run(['nm', '-S', '--size-sort', '-t', 'd', *map(str, objects)], stdout=f)

        print(f"[{arm_name}] {c['name']}: compile cost x{rounds}", flush=True)
        full, meta = [], []
        for r in range(rounds):
            d = cdir / 'cost' / f'full-{r}'
            row = perf_compile(library_cmd(root, c, backend, d / 'lib.rlib'), d)
            if row['exit_code'] == 0:
                hashes = {p.name: sha(p) for p in unpack(d / 'lib.rlib', d / 'objects')}
                row['objects_identical_to_reference'] = hashes == ref_hashes
                shutil.rmtree(d / 'objects')
            full.append(row)
            d = cdir / 'cost' / f'metadata-{r}'
            meta.append(perf_compile(library_cmd(root, c, backend, d / 'lib.rmeta', emit='metadata'), d))
        rec['compile'] = {
            'rounds': rounds, 'full': full, 'metadata_only': meta,
            'instructions_median': med(full, 'instructions'), 'cycles_median': med(full, 'cycles'),
            'wall_median': med(full, 'wall_seconds'), 'user_median': med(full, 'user_seconds'),
            'maxrss_kib_median': med(full, 'maxrss_kib'),
            'metadata_instructions_median': med(meta, 'instructions'), 'metadata_wall_median': med(meta, 'wall_seconds'),
        }
        if rec['compile']['instructions_median'] and rec['compile']['metadata_instructions_median']:
            rec['compile']['codegen_instructions_estimate'] = rec['compile']['instructions_median'] - rec['compile']['metadata_instructions_median']

        print(f"[{arm_name}] {c['name']}: driver + corpus", flush=True)
        ddir = cdir / 'driver'
        ddir.mkdir()
        dcmd = ['rustc', TC, '--sysroot=' + CONFIG['sysroot'], '--edition=2021', str(SUITE / c['driver']), '--crate-name', 'driver']
        dcmd += CONFIG['driver_flags'] + ['--extern', f"{c['lib']}={ref / 'lib.rlib'}", '-Csave-temps', '-o', str(ddir / 'driver')]
        code, _, _ = run_logged(dcmd, ddir / 'build', timeout=1800)
        rec['driver'] = {'command': dcmd, 'exit_code': code}
        if code == 0:
            dobjs = sorted(p for p in ddir.glob('*.rcgu.o'))
            rec['driver']['objects_sha256'] = {p.name: sha(p) for p in dobjs}
            rec['driver']['local_text_bytes'] = object_sizes(dobjs)[0]['text_bytes'] if dobjs else None
            rec['driver']['executable_sha256'] = sha(ddir / 'driver')
            code, _, _ = run_logged([str(ddir / 'driver'), 'corpus'], cdir / 'corpus', timeout=600)
            text = pathlib.Path(str(cdir / 'corpus') + '.stdout')
            rec['corpus'] = {'exit_code': code, 'sha256': sha(text), 'lines': len(text.read_text().splitlines())}
            checks = {}
            for workload, iters in c['workloads']:
                code, _, _ = run_logged([str(ddir / 'driver'), 'run', workload, str(iters)], cdir / f'check-{workload}', timeout=600)
                line = pathlib.Path(str(cdir / f'check-{workload}') + '.stdout').read_text().strip()
                checks[workload] = {'exit_code': code, 'checksum': json.loads(line)['checksum'] if code == 0 and line else None}
            rec['workload_check'] = checks
        write_json(out / f"{c['name']}.json", rec)

        print(f"[{arm_name}] {c['name']}: upstream tests", flush=True)
        rec['tests'] = cargo_tests(root, c, backend, out, cdir)
        write_json(out / f"{c['name']}.json", rec)
        verify_sources(root, c)
        t = rec['tests']
        print(f"[{arm_name}] {c['name']}: done; tests " + json.dumps({k: (v.get('totals') or v.get('build_error', ['?'])[:1]) for k, v in t.items() if isinstance(v, dict)}), flush=True)


# ---------------------------------------------------------------- runtime
def runtime(root, arms):
    root = pathlib.Path(root)
    rounds = int(os.environ.get('SUITE_RUNTIME_ROUNDS', '11'))
    only = os.environ.get('SUITE_CRATES')
    (root / 'runtime').mkdir(exist_ok=True)
    for c in CONFIG['crates']:
        if only and c['name'] not in only.split(','):
            continue
        avail = [a for a in arms if (root / a / c['name'] / 'driver' / 'driver').exists()]
        out = root / 'runtime' / c['name']
        out.mkdir(exist_ok=True)
        rec = {'crate': c['name'], 'arms': avail, 'skipped_arms': [a for a in arms if a not in avail], 'rounds': rounds,
               'order': 'warm-up round -1, then rounds alternate forward/reverse arm order', 'workloads': {}}
        for workload, iters in c['workloads']:
            samples = []
            for r in range(-1, rounds):
                for a in (avail if r % 2 == 0 else list(reversed(avail))):
                    exe = root / a / c['name'] / 'driver' / 'driver'
                    p = subprocess.run([str(exe), 'run', workload, str(iters)], capture_output=True, text=True, timeout=600, env=clean_env())
                    if p.returncode != 0:
                        samples.append({'arm': a, 'round': r, 'exit_code': p.returncode})
                        continue
                    res = json.loads(p.stdout)
                    samples.append({'arm': a, 'round': r, 'exit_code': 0, 'elapsed_ns': res['elapsed_ns'], 'checksum': res['checksum']})
            wl = {'iterations': iters, 'samples': samples, 'arms': {}}
            for a in avail:
                xs = [s for s in samples if s['arm'] == a and s['round'] >= 0 and s['exit_code'] == 0]
                ns = [s['elapsed_ns'] for s in xs]
                wl['arms'][a] = {'runs': len(ns), 'failures': sum(1 for s in samples if s['arm'] == a and s['exit_code'] != 0),
                                 'median_ns': statistics.median(ns) if ns else None, 'min_ns': min(ns) if ns else None,
                                 'max_ns': max(ns) if ns else None, 'checksums': sorted({s['checksum'] for s in xs})}
            # one perf-stat profile per arm, separate from the timing samples
            for a in avail:
                exe = root / a / c['name'] / 'driver' / 'driver'
                stat = out / f'{workload}-{a}.perf.csv'
                p = subprocess.run([PERF, 'stat', '-x,', '-e', 'instructions:u,cycles:u', '-o', str(stat), '--', str(exe), 'run', workload, str(iters)],
                                   capture_output=True, text=True, timeout=600, env=clean_env())
                if p.returncode == 0:
                    for line in stat.read_text().splitlines():
                        parts = line.split(',')
                        if len(parts) > 4 and parts[0].strip().isdigit():
                            wl['arms'][a][parts[2].split(':')[0]] = int(parts[0])
            rec['workloads'][workload] = wl
            write_json(root / 'runtime' / f"{c['name']}.json", rec)
            print(f"{c['name']} {workload}: " + ', '.join(f"{a}={wl['arms'][a]['median_ns'] / 1e6 if wl['arms'][a]['median_ns'] else None:.3f}ms" if wl['arms'][a]['median_ns'] else f'{a}=fail' for a in avail), flush=True)


def calibrate(root, arm_name):
    root = pathlib.Path(root)
    for c in CONFIG['crates']:
        exe = root / arm_name / c['name'] / 'driver' / 'driver'
        if not exe.exists():
            print(c['name'], 'no driver')
            continue
        for workload, iters in c['workloads']:
            ms = []
            for _ in range(3):
                p = subprocess.run([str(exe), 'run', workload, str(iters)], capture_output=True, text=True, env=clean_env())
                ms.append(json.loads(p.stdout)['elapsed_ns'] / 1e6 if p.returncode == 0 else None)
            print(c['name'], workload, iters, ms, flush=True)


if __name__ == '__main__':
    action = sys.argv[1]
    if action == 'prepare':
        prepare(sys.argv[2])
    elif action == 'arm':
        arm(sys.argv[2], sys.argv[3], sys.argv[4])
    elif action == 'runtime':
        runtime(sys.argv[2], sys.argv[3:])
    elif action == 'calibrate':
        calibrate(sys.argv[2], sys.argv[3])
    else:
        raise SystemExit(__doc__)
