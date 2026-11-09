#!/usr/bin/env python3
"""Count one exact symbol from llvm-objdump Intel disassembly.

Counts are static instructions, not runtime retired instructions. Memory counts
cover explicit operands only; implicit call/ret/push stack traffic is excluded.
Unknown memory-operand opcodes fail instead of silently guessing access modes.
"""
import argparse
from collections import Counter
import hashlib
import json
from pathlib import Path
import re

p = argparse.ArgumentParser()
p.add_argument('--disassembly', type=Path, required=True)
p.add_argument('--symbol', required=True)
p.add_argument('--text-bytes', type=int, required=True)
p.add_argument('--out', type=Path, required=True)
a = p.parse_args()
data = a.disassembly.read_bytes()
lines = data.decode().splitlines()
header = re.compile(r'^([0-9a-f]+) <(.+)>:$')
# A full-width byte column ends directly in a tab; shorter instructions have
# space padding before that tab. Neither form requires a trailing byte space.
instruction = re.compile(r'^\s*([0-9a-f]+):\s*([0-9a-f]{2}(?: [0-9a-f]{2})*)[ \t]+(\S+)\s*(.*)$')
starts = [i for i, line in enumerate(lines) if (m := header.match(line)) and m[2] == a.symbol]
assert len(starts) == 1, f'exact symbol count: {len(starts)}'
start = starts[0]
address = int(header.match(lines[start])[1], 16)
end = address + a.text_bytes
opcodes = Counter()
counts = Counter()
byte_count = 0
for line in lines[start + 1:]:
    if header.match(line):
        break
    m = instruction.match(line)
    if not m:
        assert not re.match(r'^\s*[0-9a-f]+:', line), f'unparsed instruction: {line}'
        continue
    pc = int(m[1], 16)
    if pc >= end:
        break  # Alignment padding is outside the ELF symbol size.
    size = len(m[2].split())
    assert pc == address + byte_count and pc + size <= end
    byte_count += size
    op, args = m[3], m[4].split('#', 1)[0].strip()
    operands = [x.strip() for x in args.split(',')]
    memory = [i for i, operand in enumerate(operands) if '[' in operand]
    opcodes[op] += 1
    counts['instructions'] += 1
    counts['mov_exact'] += op == 'mov'
    counts['mov_family'] += op.startswith('mov')
    counts['rbp_memory_mov_exact'] += op == 'mov' and any(re.search(r'\brbp\b', operands[i]) for i in memory)
    counts['conditional_branches'] += (op.startswith('j') and op != 'jmp') or op.startswith('loop')
    counts['unconditional_branches'] += op == 'jmp'
    counts['calls'] += op == 'call'
    counts['returns'] += op.startswith('ret')
    read = write = False
    if memory and op not in ('lea', 'nop'):
        if op.startswith(('mov', 'cmov')):
            write, read = 0 in memory, any(i > 0 for i in memory)
        elif op.startswith('set') or op == 'pop':
            write = True
        elif op in ('cmp', 'test', 'push', 'call', 'jmp', 'mul', 'imul', 'div', 'idiv'):
            read = True
        elif op in ('add', 'adc', 'sub', 'sbb', 'and', 'or', 'xor', 'inc', 'dec', 'neg', 'not', 'shl', 'shr', 'sar', 'rol', 'ror', 'xchg', 'cmpxchg'):
            read, write = True, 0 in memory
        else:
            raise AssertionError(f'unknown explicit-memory semantics: {line}')
    counts['explicit_memory_read_instructions'] += read
    counts['explicit_memory_write_instructions'] += write
assert byte_count == a.text_bytes, (byte_count, a.text_bytes)
record = {
    'symbol': a.symbol,
    'disassembly_sha256': hashlib.sha256(data).hexdigest(),
    'text_bytes': byte_count,
    'scope': 'static exact ELF symbol; explicit memory operands only, implicit stack traffic excluded; mov_exact excludes movzx/movsx; rbp mov is a frame-memory proxy, not proven spill traffic',
    'counts': dict(counts),
    'opcodes': dict(sorted(opcodes.items())),
}
a.out.write_text(json.dumps(record, indent=2) + '\n')
print(json.dumps(record, indent=2))
