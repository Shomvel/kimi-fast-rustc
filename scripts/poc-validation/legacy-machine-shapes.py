#!/usr/bin/env python3
"""Audit original machine-shape obligations against an existing Rust corpus run.

No compilation, source parsing, or reference evaluation occurs here. Disassembly
is bounded by ELF symbol sizes, so archive/section padding is never an instruction
of the tested function. Unsatisfied legacy shapes are reported as gaps, not
silently weakened into successful numerical checks.
"""
import argparse
import hashlib
import json
from pathlib import Path
import re
import subprocess


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def symbols(objdump, library, output):
    commands = [
        [objdump, '-t', '--demangle', str(library)],
        [objdump, '-d', '--demangle', '--x86-asm-syntax=intel', str(library)],
    ]
    texts = []
    for command, suffix in zip(commands, ['symbols', 'assembly']):
        result = subprocess.run(command, capture_output=True, text=True, check=True)
        (output / (library.stem + '.' + suffix + '.txt')).write_text(result.stdout)
        texts.append(result.stdout)
    sizes = {}
    for line in texts[0].splitlines():
        m = re.match(r'^([0-9a-f]+)\s+\S*\s*F\s+\S+\s+([0-9a-f]+)\s+(.+)$', line)
        if m:
            sizes[m[3]] = (int(m[1], 16), int(m[2], 16))
    result = {}
    active = None
    for line in texts[1].splitlines():
        header = re.match(r'^([0-9a-f]+) <(.+)>:$', line)
        if header:
            active = header[2] if header[2] in sizes else None
            if active:
                result[active] = []
            continue
        instruction = re.match(r'^\s*([0-9a-f]+):\s+((?:[0-9a-f]{2}\s+)+)\s*(\S+)(?:\s+(.*))?$', line)
        if active and instruction:
            start, size = sizes[active]
            address = int(instruction[1], 16)
            if start <= address < start + size:
                result[active].append({
                    'address': address, 'bytes': instruction[2].split(),
                    'op': instruction[3], 'args': (instruction[4] or '').strip(),
                })
    return result


def obligations(test, case, instructions):
    # LLVM versions/platforms choose decimal or hexadecimal operand spelling.
    # Normalize a copy for predicates while preserving raw disassembly evidence.
    instructions = [{**i, 'args': re.sub(r'(?<![\w])0x[0-9a-f]+\b',
                                        lambda m: str(int(m[0], 16)), i['args'])}
                    for i in instructions]
    rows = []
    def check(label, passed):
        rows.append({'assertion': label, 'passed': bool(passed)})
    def has(op, args=None):
        return any(i['op'] == op and (args is None or re.search(args, i['args'])) for i in instructions)
    def imm(op, value):
        return has(op, rf', {value}$')
    def no_constant_moves(value=None):
        pattern = rf', {value}$' if value is not None else r', -?(?:0x[0-9a-f]+|[0-9]+)$'
        return not any(i['op'] in ('mov', 'movabs') and re.search(pattern, i['args']) for i in instructions)
    def sequence(predicates):
        return any(all(p(i) for p, i in zip(predicates, instructions[n:]))
                   for n in range(len(instructions) - len(predicates) + 1))
    def operation(name, argument=None):
        return lambda i: i['op'] == name and (argument is None or re.search(argument, i['args']))
    def has_spill_access():
        # Callee-save slots are frame space, not allocation spill slots.
        return any(i['op'] == 'mov' and re.search(r'\[rbp - [0-9]+\]', i['args'])
                   and not re.search(r'^(?:rbx|r1[2-5]),|, (?:rbx|r1[2-5])$', i['args'])
                   for i in instructions)
    def reduced(shift):
        check('no signed division or cqo', not has('idiv') and not has('cqo'))
        check('contiguous sar63 / shr(64-shift) / add / sar(shift)', sequence([
            operation('sar', r', 63$'), operation('shr', rf', {64-shift}$'),
            operation('add'), operation('sar', rf', {shift}$'),
        ]))
    if test == 'actual_bool_results_keep_setcc_and_zero_extension':
        check('sete retained', has('sete'))
        check('bool zero extension retained', has('movzx'))
    elif test == 'shared_comparison_still_materializes_its_bool_value':
        check('shared signed-greater boolean materialized', has('setg'))
    elif test.startswith('left_constant_') and test.endswith('_uses_rhs_immediate'):
        op, value = {'addition': ('add', 5), 'multiplication': ('imul', 3), 'bitwise_and': ('and', 7)}[
            test[len('left_constant_'):-len('_uses_rhs_immediate')]]
        check(f'{op} right immediate {value}', imm(op, value))
        check('no constant register materialization', no_constant_moves())
    elif test == 'subtraction_keeps_the_constant_on_the_left':
        check('left 42 materialized', not no_constant_moves(42))
    elif test == 'left_constants_do_not_materialize_a_register':
        check('cmp immediate 2', imm('cmp', 2))
        check('2 not materialized in register', no_constant_moves(2))
    elif test == 'early_if_return_skips_the_function_tail':
        check('two physical returns', sum(i['op'] == 'ret' for i in instructions) == 2)
    elif test == 'arithmetic_uses_immediate_operands_without_constant_moves':
        check('add immediate 5', imm('add', 5))
        check('no constant register materialization', no_constant_moves())
    elif test == 'bitwise_multiply_and_compare_support_immediates':
        for op, value in ([('cmp', 42)] if case['id'].endswith('__compare') else [('and', 7), ('imul', 3), ('xor', 15)]):
            check(f'{op} immediate {value}', imm(op, value))
    elif test == 'large_constants_still_use_register_materialization':
        check('large immediate materialized', not no_constant_moves())
    elif test == 'immediate_shifts_use_count_encodings_without_cl':
        check('shl immediate 3', imm('shl', 3))
        check('no move to rcx', not has('mov', r'^rcx,'))
        check('no cl count', not any(re.search(r', cl$', i['args']) for i in instructions))
    elif test == 'immediate_count_is_masked_to_six_bits':
        count = 63 if case['id'].endswith('__0') else 1
        check(f'shl immediate {count}', imm('shl', count))
    elif test in ('shifts_survive_spills_and_loop_phi_values', 'loop_survives_register_pressure'):
        check('stack spill access beyond callee-save storage', has_spill_access())
    elif test == 'variable_shift_count_does_not_make_an_r11_rcx_round_trip':
        check('no r11-to-rcx count round trip', not has('mov', r'^rcx, r11$'))
        check('variable shift uses r10', has('shl', r'^r10, cl$'))
    elif test == 'positive_powers_avoid_idiv_and_round_toward_zero':
        reduced(int(case['id'].rsplit('__', 1)[1]).bit_length() - 1)
    elif test in ('two_to_63_bit_pattern_remains_signed_division_by_a_negative', 'dynamic_divisors_keep_all_signed_division_traps'):
        check('signed division retained', has('idiv'))
    elif test == 'reduced_division_handles_spilled_inputs_and_results':
        reduced(2)
        check('stack spill access beyond callee-save storage', has_spill_access())
        if case['id'].endswith('__early'):
            check('final shift immediately stores to spill', sequence([
                operation('sar', r', 2$'), operation('mov', r'^qword ptr \[rbp - [0-9]+\],'),
            ]))
        else:
            indexes = [n for n, i in enumerate(instructions) if i['op'] == 'sar' and i['args'].endswith(', 63')]
            check('spill load within two instructions before sign shift', any(
                any(i['op'] == 'mov' and re.search(r', qword ptr \[rbp - [0-9]+\]$', i['args'])
                    for i in instructions[max(0, n-2):n]) for n in indexes))
    elif test in ('multiplication_by_power_of_two_becomes_left_shift', 'power_of_two_on_the_left_also_becomes_left_shift'):
        check('left shift present', has('shl'))
        check('multiply removed', not has('imul'))
    elif test == 'unsigned_division_by_power_of_two_becomes_logical_shift':
        check('logical right shift present', has('shr'))
        check('unsigned divide removed', not has('div'))
    elif test == 'signed_power_division_uses_shifts_and_non_power_multiplication_is_unchanged':
        if case['id'].endswith('__multiply'):
            check('non-power multiply retained', has('imul'))
        else:
            check('arithmetic right shift present', has('sar'))
            check('signed divide removed', not has('idiv'))
    elif test == 'native_compare_and_direct_branch_use_test_for_zero':
        if case['id'].endswith('_branch'):
            check('adjacent test / conditional branch', sequence([
                operation('test'), lambda i: i['op'].startswith('j') and i['op'] != 'jmp',
            ]))
            check('no materialized comparison bool', not any(i['op'].startswith('set') for i in instructions))
        else:
            variant = case['id'].split('__')[-1].removesuffix('_compare')
            condition = {'i64_lt': 'setl', 'i64_le': 'setle', 'i64_gt': 'setg', 'i64_ge': 'setge',
                         'u64_lt': 'setb', 'u64_le': 'setbe', 'u64_gt': 'seta', 'u64_ge': 'setae',
                         'i64_eq': 'sete', 'i64_ne': 'setne'}[variant]
            check(f'adjacent test / {condition}', sequence([operation('test'), operation(condition)]))
    elif test == 'native_zero_compare_example_evaluates_to_42':
        check('test instruction present', has('test'))
    elif test == 'native_zero_return_uses_xor_and_saves_four_bytes':
        expected = '55 48 89 e5 48 31 c9 48 89 c8 c9 c3'.split()
        check('original exact zero-return bytes and registers', [b for i in instructions for b in i['bytes']] == expected)
    elif test == 'native_branch_example_uses_zero_idioms_and_returns_42':
        check('exactly one self xor', sum(i['op'] == 'xor' and len(i['args'].split(', ')) == 2
              and i['args'].split(', ')[0] == i['args'].split(', ')[1] for i in instructions) == 1)
    elif test == 'native_loop_preserves_conditions_and_returns_42':
        check('adjacent cmp / conditional branch', sequence([
            operation('cmp'), lambda i: i['op'].startswith('j') and i['op'] != 'jmp',
        ]))
    else:
        raise ValueError('unmapped shape obligation: ' + test)
    return rows


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--run', type=Path, required=True, help='legacy-corpus-run output with summary.json')
    parser.add_argument('--out', type=Path, required=True)
    parser.add_argument('--objdump', default='llvm-objdump', help='LLVM objdump with Rust demangling')
    parser.add_argument('--arm', default='semantic-on')
    parser.add_argument('--require-all', action='store_true', help='exit nonzero when any original shape remains unmet/unmeasured')
    args = parser.parse_args()
    args.out.mkdir(parents=True, exist_ok=False)
    inventory = json.loads((Path(__file__).parent/'fixtures/legacy-corpus/migration-status.json').read_text())
    targets = {(f['path'], t['test']): t for f in inventory['files'] for t in f['tests'] if 'machine_shape_disposition' in t}
    rows = {key: {'path': key[0], 'test': key[1], 'cases': []} for key in targets}
    run = json.loads((args.run/'summary.json').read_text())
    for suite in run['suites']:
        manifest = suite['source_manifest']
        candidates = [c for c in manifest['cases'] if any((r['path'], r['test']) in targets for r in c['source_refs'])]
        if not candidates:
            continue
        library = args.run/suite['suite']/args.arm/('lib'+manifest['crate_name']+'.rlib')
        expected_sha = suite['arms'][args.arm]['library_sha256']
        if digest(library) != expected_sha:
            raise ValueError('library does not match original run receipt: ' + str(library))
        selected = symbols(args.objdump, library, args.out)
        for case in candidates:
            name = manifest['crate_name']+'::'+case['module']+'::test_fn'
            matches = [value for symbol, value in selected.items() if symbol == name or symbol.startswith(name+'::h')]
            if len(matches) != 1:
                raise ValueError(f'expected exactly one emitted function {name}, got {len(matches)}')
            for ref in case['source_refs']:
                key = ref['path'], ref['test']
                if key not in targets:
                    continue
                if key[1] == 'acyclic_loop_phi_edges_do_not_allocate_temporaries':
                    checks = []
                    status = 'requires-virtual-instruction-test'
                else:
                    checks = obligations(key[1], case, matches[0])
                    status = 'satisfied' if all(c['passed'] for c in checks) else 'shape-gap'
                rows[key]['cases'].append({'id': case['id'], 'symbol': name, 'status': status,
                    'source_sha256': case['source_sha256'], 'library_sha256': expected_sha,
                    'instructions': matches[0], 'assertions': checks})
    for key, row in rows.items():
        expected = {name.split('/', 1)[1] for name in targets[key]['replacement_cases']}
        measured = {case['id'] for case in row['cases']}
        row['missing_cases_in_run'] = sorted(expected-measured)
        row['status'] = ('unmeasured' if expected-measured else
                         'satisfied' if row['cases'] and all(c['status'] == 'satisfied' for c in row['cases']) else
                         'shape-gap' if any(c['status'] == 'shape-gap' for c in row['cases']) else
                         'requires-virtual-instruction-test')
    result = {'scope': 'Original machine-shape obligations on verified existing real-Rust corpus libraries; numerical/trap evidence remains in the original runner receipt. No compilation or benchmark was performed.',
              'audit_script_sha256': digest(Path(__file__)),
              'operand_normalization': 'hexadecimal integer tokens normalized to decimal only for assertions; raw instructions retained',
              'run_summary_sha256': digest(args.run/'summary.json'), 'arm': args.arm,
              'results': list(rows.values())}
    (args.out/'summary.json').write_text(json.dumps(result, indent=2)+'\n')
    counts = {status: sum(row['status'] == status for row in rows.values()) for status in sorted({r['status'] for r in rows.values()})}
    print(json.dumps(counts, indent=2))
    if args.require_all and counts.get('satisfied', 0) != len(rows):
        raise SystemExit(1)


if __name__ == '__main__':
    main()
