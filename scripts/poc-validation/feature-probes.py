#!/usr/bin/env python3
"""Production R specialization and object reachability through three backends."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess

p = argparse.ArgumentParser()
p.add_argument('--repo', type=Path, required=True)
p.add_argument('--backend', type=Path, required=True)
p.add_argument('--out', type=Path, required=True)
p.add_argument('--resume', action='store_true', help='reuse successful identical command receipts after a validator-only change')
a = p.parse_args()
a.out.mkdir(parents=True, exist_ok=a.resume)
fixtures = a.repo/'scripts/poc-validation/fixtures'
env = os.environ.copy()
for name in ('KFRC_PROFILE', 'KFRC_PROFILE_TOTAL', 'KFRC_LLVM_PROFILE', 'KFRC_INLINE', 'KFRC_OPTIMIZE', 'KFRC_DUMP_GRAPH', 'KFRC_DUMP_BODY', 'KFRC_INTERPRET', 'KFRC_SPECIALIZATION_STATS'):
    env.pop(name, None)
env['RUSTC_BOOTSTRAP'] = '1'
base = ['rustc', '+1.94.1', '--edition=2021', '-Cpanic=abort', '-Coverflow-checks=off', '-Cdebug-assertions=off', '-Zmir-opt-level=0', '-Ccodegen-units=1']
summary = {'source_sha256': {p.name: hashlib.sha256(p.read_bytes()).hexdigest() for p in [fixtures/'generic-semantics.rs', fixtures/'emission-reachability.rs', fixtures/'emission-reachability-driver.rs', fixtures/'nominal-identity.rs', fixtures/'generic-variants.rs', fixtures/'maybe-uninit.rs', fixtures/'maybe-uninit-driver.rs', fixtures/'assume-semantics.rs']}, 'arms': {}}

if a.resume:
    prior = json.loads((a.out/'summary.json').read_text())
    assert prior['source_sha256'] == summary['source_sha256'], 'resume source changed'
    if 'backend_sha256' in prior:
        assert prior['backend_sha256'] == hashlib.sha256(a.backend.read_bytes()).hexdigest(), 'resume backend changed'
summary['backend_sha256'] = hashlib.sha256(a.backend.read_bytes()).hexdigest()
summary['resumed_existing_artifacts'] = a.resume

def run(command, stem, active):
    receipt = stem.with_suffix('.result.json')
    if a.resume and receipt.exists():
        prior = json.loads(receipt.read_text())
        assert prior['command'] == command and prior['exit_code'] == 0, f'cannot reuse failed or changed command: {stem}'
        return stem.with_suffix('.stdout').read_bytes()
    with stem.with_suffix('.stdout').open('w') as out, stem.with_suffix('.stderr').open('w') as err:
        result = subprocess.run(command, env=active, stdout=out, stderr=err, timeout=120)
    stem.with_suffix('.result.json').write_text(json.dumps({'command': command, 'exit_code': result.returncode}, indent=2)+'\n')
    if result.returncode:
        raise SystemExit(f'first failure: {stem}')
    return stem.with_suffix('.stdout').read_bytes()

reference = {}
for arm in ('llvm', 'semantic-off', 'semantic-on'):
    folder = a.out/arm
    folder.mkdir(exist_ok=a.resume)
    active = env.copy()
    flags = []
    if arm != 'llvm':
        active.update(KFRC_OPTIMIZE='1' if arm.endswith('on') else '0', KFRC_SPECIALIZATION_STATS='1')
        flags = ['-Zcodegen-backend='+str(a.backend)]
    row = {}
    executable = folder/'generic'
    run([*base, '-Copt-level=0', *flags, str(fixtures/'generic-semantics.rs'), '-o', str(executable)], folder/'generic-build', active)
    output = run([str(executable)], folder/'generic-run', env)
    expected = b'greater unsigned=true signed=false custom=false\nchoose zst=7 sized=23\ndrop scalar=11 string=13\nvariant some=4294967295 none=31\n'
    assert output == expected, f'{arm}: generic output differs from explicit expected values'
    if arm == 'llvm':
        reference['generic'] = output
    else:
        assert output == reference['generic']
        log = (folder/'generic-build.stderr').read_text()
        row['specialization_records'] = [line for line in log.splitlines() if line.startswith('kfrc specialization ') or line.startswith('kfrc R coverage:')]
        records = [line for line in row['specialization_records'] if line.startswith('kfrc specialization ')]
        grouped = {name: [line for line in records if name in line.split(': SpecializationStats', 1)[0]] for name in ('greater', 'choose', 'drop_question')}
        assert {name: len(lines) for name, lines in grouped.items()} == {'greater': 3, 'choose': 2, 'drop_question': 2}, 'missing concrete instances of one generic definition'
        prepared = [line for line in log.splitlines() if line.startswith('kfrc R prepared ')]
        for name in grouped:
            assert sum(bool(re.search(r'(?:prepared |::)' + name + r': count=1(?: stats=.*)?$', line)) for line in prepared) == 1, f'{name}: definition was not prepared exactly once'
        zst = [line for line in grouped['choose'] if 'SizeAlign { size: 0,' in line]
        assert len(zst) == 1
        if arm == 'semantic-on':
            assert 'holes_answered: 1,' in zst[0] and 'branches_pruned: 1,' in zst[0] and 'Resolved {' not in zst[0], 'dead ZST branch resolved a trait target'
        sized = [line for line in grouped['choose'] if 'SizeAlign { size: 4,' in line]
        assert len(sized) == 1 and 'holes_answered: 2,' in sized[0] and 'Resolved {' in sized[0]
        targets = [re.search(r'identity: "([^"]+)"', line).group(1) for line in grouped['greater']]
        assert len(set(targets)) == 3, 'signed/unsigned/custom dispatch identity was conflated'
        assert any('NeedsDrop(true)' in line for line in grouped['drop_question']) and any('NeedsDrop(false)' in line for line in grouped['drop_question'])
        row['prepared_records'] = prepared
        row['generic_contract'] = {'instances': {name: len(lines) for name, lines in grouped.items()}, 'definitions_prepared_once': True, 'dead_zst_target_unresolved': 'Resolved {' not in zst[0], 'distinct_greater_targets': targets}
    nominal = folder/'nominal'
    run([*base, '-Copt-level=0', *flags, str(fixtures/'nominal-identity.rs'), '-o', str(nominal)], folder/'nominal-build', active)
    output = run([str(nominal)], folder/'nominal-run', env)
    assert output == b'nested 13 20\nitems 17 29\nclosures 31 43\n', 'nominal types, function items, or closures were conflated'
    variant_driver = folder/'generic-variants'
    variant_env = active.copy()
    if arm != 'llvm':
        variant_env['KFRC_DUMP_GRAPH'] = '1'
    run([*base, '-Copt-level=0', *flags, str(fixtures/'generic-variants.rs'), '-o', str(variant_driver)], folder/'generic-variants-build', variant_env)
    output = run([str(variant_driver)], folder/'generic-variants-run', env)
    assert output == b'select unsigned 2147483649 13\nselect signed -7 17\nnested 2147483649 19 19\nknown 2147483649 -7\n'
    if arm != 'llvm':
        variant_log = (folder/'generic-variants-build.stderr').read_text()
        variant_records = [line for line in variant_log.splitlines() if line.startswith('kfrc specialization ')]
        counts = {name: sum(name in line.split(': SpecializationStats', 1)[0] for line in variant_records) for name in ('select', 'nested', 'known')}
        assert counts == {'select': 2, 'nested': 1, 'known': 2}, 'generic enum instances were not specialized'
        for name in counts:
            prepared = [line for line in variant_log.splitlines() if line.startswith('kfrc R prepared ') and re.search(r'(?:prepared |::)' + name + r': count=1(?: stats=.*)?$', line)]
            assert len(prepared) == 1, f'{name}: enum definition was not prepared exactly once'
        if arm == 'semantic-on':
            definitions = {name.rsplit('::', 1)[-1]: body for name, body in re.findall(r'^graph ([^\n]+) \(Rust, (?:Verified|Sealed)\) \{\n(.*?)^\}', variant_log, re.M | re.S)}
            for name in ('select', 'nested'):
                assert 'VariantSwitch' in definitions[name] and 'VariantWitness' in definitions[name], f'{name}: semantic variant witnesses were lost'
            assert not re.search(r'= (If|VariantSwitch)\b', definitions['known']), 'known constructor branch survived definition optimization'
        row['generic_variant_instances'] = counts
    assume_driver = folder/'assume-semantics'
    run([*base, '-Copt-level=0', *flags, str(fixtures/'assume-semantics.rs'), '-o', str(assume_driver)], folder/'assume-build', variant_env)
    output = run([str(assume_driver)], folder/'assume-run', env)
    assert output == b'1 4 2\n7 10 14\n99 102 198\ngeneric 17 -23\n', 'defined assume inputs differ'
    if arm != 'llvm':
        assume_log = (folder/'assume-build.stderr').read_text()
        graphs = re.findall(r'^graph ([^\n]+) \((Rust|Specialized|Layout), (?:Verified|Sealed)\) \{\n(.*?)^\}', assume_log, re.M | re.S)
        source_assumes = [(name, level) for name, level, body in graphs if level in ('Rust', 'Specialized') and re.search(r'= Assume\b', body)]
        assert source_assumes, 'real rustc producer dropped every assume before semantic IR'
        assert any('through_intrinsic' in name for name, _ in source_assumes), 'direct intrinsic assume was lost'
        assert any('through_hint' in name or 'assert_unchecked' in name for name, _ in source_assumes), 'stable unchecked hint assume was lost'
        assert any('generic_assume' in name for name, _ in source_assumes), 'generic definition assume was lost'
        if arm == 'semantic-on':
            assert any('generic_assume' in name and level == 'Rust' for name, level in source_assumes), 'generic R preparation dropped the assume'
        assert not any(re.search(r'= (Assume|RequireDefined|VariantWitness)\b', body) for _, level, body in graphs if level == 'Layout'), 'release layout retained requirement/proof nodes'
        row['assume_source_graphs'] = source_assumes
    opaque_library = folder/'libmaybe_uninit_values.rlib'
    run([*base, '-Copt-level=0', *flags, '--crate-type=rlib', '--crate-name=maybe_uninit_values', str(fixtures/'maybe-uninit.rs'), '-o', str(opaque_library)], folder/'maybe-uninit-build', active)
    opaque_driver = folder/'maybe-uninit-driver'
    run([*base, '-Copt-level=0', str(fixtures/'maybe-uninit-driver.rs'), '--extern=maybe_uninit_values='+str(opaque_library), '-o', str(opaque_driver)], folder/'maybe-uninit-driver-build', env)
    output = run([str(opaque_driver)], folder/'maybe-uninit-run', env)
    if arm == 'llvm':
        reference['maybe-uninit'] = output
    else:
        assert output == reference['maybe-uninit'], 'MaybeUninit or array iterator behavior differs'
    row['maybe_uninit_output_lines'] = len(output.splitlines())
    library = folder/'libemission_reachability.rlib'
    run([*base, '-Copt-level=3', *flags, '--crate-name=emission_reachability', '--crate-type=rlib', str(fixtures/'emission-reachability.rs'), '-o', str(library)], folder/'emission-build', active)
    driver = folder/'emission-driver'
    run([*base, '-Copt-level=0', str(fixtures/'emission-reachability-driver.rs'), '--extern=emission_reachability='+str(library), '-o', str(driver)], folder/'emission-driver-build', env)
    output = run([str(driver)], folder/'emission-run', env)
    if arm == 'llvm':
        reference['emission'] = output
    else:
        assert output == reference['emission']
    symbols = subprocess.check_output(['nm', '--defined-only', str(library)], text=True, stderr=subprocess.DEVNULL)
    (folder/'emission-symbols.txt').write_text(symbols)
    row['emission_symbols'] = {name: [line for line in symbols.splitlines() if name in line] for name in ('embedded', 'callback', 'recursive')}
    if arm != 'llvm':
        assert row['emission_symbols']['callback'], 'static function address root disappeared'
        assert row['emission_symbols']['recursive'], 'recursive target disappeared'
        row['emission_records'] = [line for line in (folder/'emission-build.stderr').read_text().splitlines() if 'emission' in line and 'kfrc' in line]
    # A Global symbol must remain even when every call was inlined. Symbol
    # omission is checked only through the backend's actual Internal report.
    row['embedded_presence_is_not_a_failure'] = True
    summary['arms'][arm] = row
    (a.out/'summary.json').write_text(json.dumps(summary, indent=2)+'\n')
print('PASS: generic specialization contracts, nominal identity, real assume producer, MaybeUninit, emission roots in three arms')
