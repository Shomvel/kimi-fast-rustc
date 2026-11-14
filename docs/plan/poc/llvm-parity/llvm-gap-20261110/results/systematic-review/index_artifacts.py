"""Produce review inventories and exact ELF64 file-byte accounting.

Usage: python3 index_artifacts.py RESULTS OUTPUT_DIRECTORY
Only little-endian ELF64 relocatable objects from this experiment are accepted.
"""
import collections
import csv
import hashlib
import json
from pathlib import Path
import struct
import sys

root, output = map(Path, sys.argv[1:])
output.mkdir(parents=True, exist_ok=True)
functions = json.loads((root / 'static-review/functions.json').read_text())
columns = ['arm', 'crate', 'object', 'symbol', 'bytes', 'instructions', 'calls', 'branches', 'object_sha256']
with (output / 'function-inventory.csv').open('w') as f:
    writer = csv.DictWriter(f, columns, extrasaction='ignore', lineterminator='\n')
    writer.writeheader()
    writer.writerows(functions)

totals = collections.defaultdict(collections.Counter)
for row in functions:
    total = totals[row['crate'], row['arm']]
    total['function_bodies'] += 1
    for key in ['bytes', 'instructions', 'calls', 'branches']:
        total[key] += row[key]
static = [{'crate': c, 'arm': a, **v} for (c, a), v in sorted(totals.items())]
(output / 'static-summary.json').write_text(json.dumps(static, indent=2) + '\n')

groups = collections.defaultdict(collections.Counter)
objects = json.loads((root / 'static-review/objects.json').read_text())
for item in objects:
    path = root / 'suite' / item['arm'] / item['crate'] / 'ref/objects' / item['object']
    data = path.read_bytes()
    assert hashlib.sha256(data).hexdigest() == item['sha256'], path
    assert data[:6] == b'\x7fELF\x02\x01', path
    header = struct.unpack_from('<16sHHIQQQIHHHHHH', data)
    _, kind, _, _, _, phoff, shoff, _, ehsize, phentsize, phnum, shentsize, shnum, shstrndx = header
    assert kind == 1 and phnum == 0 and shnum > 0 and shentsize == 64, path
    sections = [struct.unpack_from('<IIQQQQIIQQ', data, shoff + i * shentsize) for i in range(shnum)]
    names = sections[shstrndx]
    strings = data[names[4]:names[4] + names[5]]
    parts = collections.Counter(file_bytes=len(data), object_count=1)
    intervals = [(0, ehsize), (shoff, shoff + shentsize * shnum)]
    parts['elf_headers'] = ehsize + shentsize * shnum
    for index, section in enumerate(sections):
        nameoff, ty, flags, _, offset, size, *_ = section
        if index == 0 or ty == 8 or not size:  # NULL / NOBITS occupy no file payload.
            continue
        name = strings[nameoff:strings.index(b'\0', nameoff)].decode()
        category = ('relocations' if ty in (4, 9) else
                    'symbols' if ty in (2, 11) else
                    'strings' if ty == 3 else
                    'code' if flags & 4 else
                    'rodata' if name.startswith('.rodata') else 'other_payload')
        parts[category] += size
        intervals.append((offset, offset + size))
    intervals.sort()
    assert all(a[1] <= b[0] for a, b in zip(intervals, intervals[1:])), path
    accounted = sum(end - start for start, end in intervals)
    assert accounted <= len(data), path
    parts['padding_and_gaps'] = len(data) - accounted
    assert sum(v for k, v in parts.items() if k not in ('file_bytes', 'object_count')) == len(data)
    groups[item['crate'], item['arm']].update(parts)
rows = [{'crate': c, 'arm': a, **v} for (c, a), v in sorted(groups.items())]
(output / 'object-sections.json').write_text(json.dumps(rows, indent=2) + '\n')
print(f'Indexed {len(functions)} function bodies and hash-verified {len(objects)} objects.')
