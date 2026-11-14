#!/usr/bin/env bash
set -euo pipefail
backend=$(realpath "${1:?backend dylib}")
work=${2:?fresh result directory}
here=$(cd "$(dirname "$0")/fixtures" && pwd)
mkdir "$work"
work=$(realpath "$work")
export RUSTC_BOOTSTRAP=1
flags=(--edition=2021 -Cpanic=abort -Copt-level=3 -Zmir-opt-level=0 -Ccodegen-units=16 -Clto=off -Cembed-bitcode=no)
if [ -n "${KFRC_SYSROOT:-}" ]; then flags+=(--sysroot="$KFRC_SYSROOT"); fi

# Diagnostics have a separate build, so normal machine artifacts do not rely
# on a diagnostic compilation. All three runtime arms share one LLVM caller.
for arm in llvm kfrc-off kfrc-on kfrc-diagnostic; do
    mkdir "$work/$arm"
    backend_flags=()
    mode=0
    if [ "$arm" != llvm ]; then backend_flags=(-Zcodegen-backend="$backend"); fi
    if [ "$arm" = kfrc-on ] || [ "$arm" = kfrc-diagnostic ]; then mode=1; fi
    stats=0
    if [ "$arm" = kfrc-diagnostic ]; then stats=1; fi
    (
        unset KFRC_CGSCC_STATS KFRC_SPECIALIZATION_STATS KFRC_DUMP_GRAPH KFRC_DUMP_BODY
        if [ "$stats" = 1 ]; then export KFRC_CGSCC_STATS=1; fi
        export KFRC_CGSCC="$mode"
        set -x
        rustc +"${KFRC_TOOLCHAIN:-1.94.1}" "${flags[@]}" "${backend_flags[@]}" --crate-name=cgscc --crate-type=rlib "$here/cgscc.rs" --out-dir="$work/$arm"
    ) > "$work/$arm/build.log" 2>&1
    objdump -dr --no-show-raw-insn -Mintel "$work/$arm/libcgscc.rlib" > "$work/$arm/disassembly.txt" 2> "$work/$arm/objdump.stderr"
done

python3 - "$backend" "$here" "$work" "$0" <<'PY'
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys

backend, here, work, script = map(Path, sys.argv[1:])
sha = lambda path: hashlib.sha256(Path(path).read_bytes()).hexdigest()
env = {key: value for key, value in os.environ.items() if not key.startswith('KFRC_')}
env['RUSTC_BOOTSTRAP'] = '1'
caller = work / 'caller'
caller.mkdir()
link_receipt = caller / 'link-receipt.json'
linker = shutil.which('cc')
assert linker, 'cc is required for one-caller link replay'
capture = caller / 'capture-linker.py'
capture.write_text(
    '#!/usr/bin/env python3\n'
    'import json, os, subprocess, sys\n'
    'from pathlib import Path\n'
    'args = sys.argv[1:]\n'
    'linker = os.environ["CGSCC_REAL_LINKER"]\n'
    'Path(os.environ["CGSCC_LINK_RECEIPT"]).write_text(json.dumps({"linker": linker, "args": args}, indent=2) + "\\n")\n'
    'raise SystemExit(subprocess.run([linker, *args]).returncode)\n'
)
capture.chmod(0o755)
flags = ['--edition=2021', '-Cpanic=abort', '-Copt-level=3', '-Zmir-opt-level=0',
         '-Ccodegen-units=1', '-Clto=off', '-Cembed-bitcode=no', '-Csave-temps']
if os.environ.get('KFRC_SYSROOT'):
    flags.append('--sysroot=' + os.environ['KFRC_SYSROOT'])
library = work / 'llvm/libcgscc.rlib'
command = ['rustc', '+' + os.environ.get('KFRC_TOOLCHAIN', '1.94.1'), *flags,
           '--crate-name=cgscc_driver', str(here / 'cgscc-driver.rs'),
           '--extern=cgscc=' + str(library), '-Clinker=' + str(capture),
           '-o', str(caller / 'original-driver')]
with (caller / 'build.log').open('w') as log:
    subprocess.run(command, cwd=caller, env=dict(env, CGSCC_REAL_LINKER=linker,
                   CGSCC_LINK_RECEIPT=str(link_receipt)), stdout=log, stderr=log, check=True)
link = json.loads(link_receipt.read_text())
args = link['args']
assert str(library) in args and args.count('-o') == 1, 'link must identify the exact fixture rlib'
objects = {str(Path(arg)): sha(arg) for arg in args if arg.endswith('.o') and Path(arg).is_file()}
assert objects and any('.rcgu.o' in name for name in objects), 'complete LLVM caller objects required'
record = {
    'schema': 1, 'status': 'running', 'backend_sha256': sha(backend),
    'fixture_sha256': sha(here / 'cgscc.rs'), 'driver_sha256': sha(here / 'cgscc-driver.rs'),
    'script_sha256': sha(script), 'caller_command': command, 'caller_object_sha256': objects,
    'contract': 'One LLVM caller compilation; replay only the exact rlib and output paths, no LTO.',
    'arms': {},
}
(work / 'summary.json').write_text(json.dumps(record, indent=2) + '\n')
for arm in ['llvm', 'kfrc-off', 'kfrc-on']:
    folder = work / arm
    arguments = [str(folder / 'libcgscc.rlib') if arg == str(library) else arg for arg in args]
    arguments[arguments.index('-o') + 1] = str(folder / 'driver')
    with (folder / 'link.log').open('w') as log:
        subprocess.run([link['linker'], *arguments], cwd=caller, env=env,
                       stdout=log, stderr=log, check=True)
    assert {name: sha(name) for name in objects} == objects, 'caller changed during relink'
    (folder / 'link-command.json').write_text(json.dumps({
        'command': [link['linker'], *arguments], 'caller_object_sha256': objects}, indent=2) + '\n')
    run = subprocess.run([str(folder / 'driver')], env=env, capture_output=True,
                         text=True, timeout=120)
    (folder / 'output.txt').write_text(run.stdout)
    (folder / 'stderr.txt').write_text(run.stderr)
    assert run.returncode == 0, (arm, run.returncode, run.stderr)
    record['arms'][arm] = {'driver_sha256': sha(folder / 'driver'),
                          'rlib_sha256': sha(folder / 'libcgscc.rlib'), 'output': run.stdout}
assert len({entry['output'] for entry in record['arms'].values()}) == 1, 'LLVM differential failed'
(work / 'summary.json').write_text(json.dumps(record, indent=2) + '\n')

def members(path):
    names = subprocess.check_output(['ar', 't', str(path)], text=True).splitlines()
    names = [name for name in names if name.endswith('.o')]
    assert names and len(set(names)) == len(names), 'object members must be unambiguous'
    return {name: hashlib.sha256(subprocess.check_output(['ar', 'p', str(path), name])).hexdigest()
            for name in names}

normal = members(work / 'kfrc-on/libcgscc.rlib')
diagnostic = members(work / 'kfrc-diagnostic/libcgscc.rlib')
assert normal == diagnostic, 'CGSCC diagnostics changed a whole object'
record['diagnostic_object_binding'] = normal
telemetry = [json.loads(line.split(' ', 1)[1]) for line in
             (work / 'kfrc-diagnostic/build.log').read_text().splitlines()
             if line.startswith('kfrc-cgscc ')]
assert telemetry, 'CGSCC per-function telemetry missing'
published = [row for row in telemetry if row.get('outcome') == 'published']
stages = [row for row in published if re.search(r'stage_[1-7]', row.get('symbol', ''))]
assert len(stages) >= 7, 'all seven real wrapper bodies must be published after callee preparation'
assert all(row['source_nodes'] > 128 and row['final_nodes'] <= 64 for row in stages), \
    'fixture must cross the old scratch/callee thresholds through cleanup'
assert all(row['source_calls'] > row['final_calls'] for row in stages), 'published wrappers must lose calls'
assert any(row.get('outcome') == 'recursive' for row in telemetry), 'recursive SCC boundary missing'
record['telemetry'] = telemetry

def functions(path):
    result = {}
    current = None
    for line in path.read_text().splitlines():
        match = re.match(r'^([0-9a-f]+) <([^>]+)>:$', line)
        if match:
            current = match.group(2)
            result.setdefault(current, [])
        elif current and re.match(r'^\s*[0-9a-f]+:', line):
            result[current].append(line.strip())
    return result

machine = {arm: functions(work / arm / 'disassembly.txt') for arm in ['llvm', 'kfrc-off', 'kfrc-on']}
metrics = {}
for arm, bodies in machine.items():
    metrics[arm] = {}
    for name in ['cgscc_chain', 'cgscc_diamond', 'cgscc_noinline_caller', 'cgscc_external_caller']:
        body = bodies.get(name)
        assert body, (arm, 'missing native body', name)
        instructions = [line for line in body if not re.search(r'R_X86_64_', line)]
        metrics[arm][name] = {
            'instructions': len(instructions),
            'calls': sum(bool(re.search(r'\bcallq?\b', line)) for line in instructions),
            'stack_ops': sum(bool(re.search(r'\b(?:push|pop|enter|leave)\b|\[(?:r|e)(?:sp|bp)', line))
                             for line in instructions),
        }
for name in ['cgscc_chain', 'cgscc_diamond']:
    assert metrics['kfrc-off'][name]['calls'] > metrics['kfrc-on'][name]['calls'], \
        (name, 'callee publication must reduce real native calls', metrics)
    assert metrics['kfrc-on'][name]['calls'] == 0, (name, 'positive chain still calls helpers')
for name, target in [('cgscc_noinline_caller', 'cgscc_noinline'),
                     ('cgscc_external_caller', 'cgscc_external_pair')]:
    assert any(target in line for line in machine['kfrc-on'][name]), (name, 'required call relocation lost')
record.update(status='passed', machine=metrics)
(work / 'summary.json').write_text(json.dumps(record, indent=2) + '\n')
print(record['arms']['kfrc-on']['output'], end='')
print('cgscc mechanism: seven wrappers published, positive native calls removed, boundaries retained')
PY
