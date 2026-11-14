"""Inspect archived ELF objects; static counts are not dynamic execution counts.

Usage: python3 analyze_hotspots.py EXTRACTED_FORMAL_SUITE --output OUTPUT_DIR
Requires LLVM-compatible objdump (Apple objdump also reads these x86 ELF files).
The input directory contains bits/CRATE/ref/objects and llvm/CRATE/ref/objects.
"""
import argparse
import collections
import hashlib
import json
import pathlib
import re
import subprocess


def inspect(path, objdump, asm_path):
    symbols = subprocess.check_output([objdump, '-t', '--demangle', str(path)], text=True)
    sizes = {}
    for line in symbols.splitlines():
        match = re.match(r'^([0-9a-f]+)\s+\S+\s+F\s+(\S+)\s+([0-9a-f]+)\s+(.+)$', line)
        if match:
            sizes[match[4].removeprefix('.hidden ')] = (match[2], int(match[1], 16), int(match[3], 16))
    asm = subprocess.check_output(
        [objdump, '-dr', '--demangle', '--x86-asm-syntax=intel', str(path)], text=True)
    asm_path.parent.mkdir(parents=True, exist_ok=True)
    asm_path.write_text(asm)
    functions = []
    current = None
    last_op = None
    for line in asm.splitlines():
        label = re.match(r'^([0-9a-f]+) <(.+)>:$', line)
        if label:
            name = label[2]
            current = None
            last_op = None
            if name in sizes:
                section, start, size = sizes[name]
                current = dict(name=name, start=start, bytes=size,
                               section=section,
                               aliases=[n for n, extent in sizes.items() if n != name and extent == sizes[name]],
                               counts=collections.Counter(), opcodes=collections.Counter(),
                               call_targets=collections.Counter(), stack_reservations=[])
                functions.append(current)
            continue
        if current is None:
            continue
        relocation = re.search(r'R_X86_64_\w+\s+(.+)$', line)
        if relocation:
            if last_op == 'call':
                current['call_targets'][re.sub(r'-0x4$', '', relocation[1])] += 1
            continue
        instruction = re.match(r'^\s*([0-9a-f]+):\s+(?:[0-9a-f]{2}\s+)+\s*([a-z][a-z0-9]*)\s*(.*)$', line)
        if not instruction:
            continue
        address, op, args = int(instruction[1], 16), instruction[2], instruction[3]
        if not current['start'] <= address < current['start'] + current['bytes']:
            continue
        last_op = op
        c = current['counts']
        c['instructions'] += 1
        current['opcodes'][op] += 1
        if op == 'call':
            c['calls'] += 1
            target = re.search(r'<(.+)>', args)
            if target and not target[1].startswith(current['name'] + '+'):
                current['call_targets'][target[1]] += 1
        if op.startswith('j'):
            c['branches'] += 1
        if '[' in args and op != 'lea':
            c['memory_operand_instructions'] += 1
            if re.search(r'\[(?:r[bs]p)(?:\s|\])', args):
                c['direct_stack_operand_instructions'] += 1
        if op in ('shl', 'shr', 'sar', 'rol', 'ror'):
            c['cl_shifts' if args.endswith(', cl') else 'non_cl_shifts'] += 1
        if op == 'sub' and args.startswith('rsp, '):
            current['stack_reservations'].append(args[5:])
    covered = {n for f in functions for n in [f['name'], *f['aliases']]}
    assert covered == set(sizes), (str(path), set(sizes) - covered)
    return functions


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('suite', type=pathlib.Path)
    parser.add_argument('--output', required=True, type=pathlib.Path)
    parser.add_argument('--objdump', default='/usr/bin/objdump')
    args = parser.parse_args()
    result = dict(objdump_version=subprocess.check_output([args.objdump, '--version'], text=True),
                  scope='Static instructions within ELF function symbol extents; stack operands are not spill counts.',
                  inputs={}, crates={})
    for arm in ('llvm', 'bits'):
        for crate in ('semver', 'base64', 'unicode-segmentation', 'humantime'):
            objects = sorted((args.suite / arm / crate / 'ref/objects').glob('*.o'))
            assert objects, (arm, crate)
            rows = []
            for path in objects:
                relative = path.relative_to(args.suite)
                result['inputs'][str(relative)] = hashlib.sha256(path.read_bytes()).hexdigest()
                functions = inspect(path, args.objdump, args.output / 'asm' / relative.with_suffix('.asm'))
                for function in functions:
                    function['object'] = str(relative)
                rows.extend(functions)
            result['crates'].setdefault(crate, {})[arm] = rows
    args.output.mkdir(parents=True, exist_ok=True)
    (args.output / 'static-hotspots.json').write_text(json.dumps(result, indent=2) + '\n')
    focus = {
        'semver': ('parse::numeric_identifier', 'parse::comparator', 'Version as core::str',
                   'Prerelease as core::cmp::Ord', 'eval::matches_impl'),
        'base64': ('internal_encode', 'internal_decode', 'decode_helper', 'decode_chunk_8', 'read_u64'),
        'unicode-segmentation': ('prev_boundary', 'is_boundary', 'next_code_point_reverse',
                                 'grapheme_category', 'NonNull<u8>>::sub'),
        'humantime': ('Parser>::parse', 'Unit as core::str', 'Rfc3339Timestamp as core::fmt',
                      'parse_rfc3339_weak'),
    }
    focused = {key: value for key, value in result.items() if key != 'crates'}
    focused['crates'] = {
        crate: {arm: [row for row in rows if any(key in row['name'] for key in focus[crate])]
                for arm, rows in arms.items()}
        for crate, arms in result['crates'].items()
    }
    (args.output / 'hotspot-evidence.json').write_text(json.dumps(focused, indent=2) + '\n')
    for crate, arms in result['crates'].items():
        print(crate)
        for arm, rows in arms.items():
            print(arm, len(rows), 'symbols', sum(x['bytes'] for x in rows), 'bytes')
            for row in sorted(rows, key=lambda r: r['bytes'], reverse=True)[:6]:
                print(row['bytes'], row['name'], dict(row['counts']))


if __name__ == '__main__':
    main()
