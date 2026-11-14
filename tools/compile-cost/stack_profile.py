#!/usr/bin/env python3
"""Attribute `perf script -F ip,sym | c++filt` call stacks (gzip) inside codegen_crate.

usage: python3 -I stack_profile.py stacks.txt.gz [--top 40] [--focus SUBSTRING]

Only samples whose stack contains `codegen_crate` (the kfrc backend window) are
counted. Prints, as shares of those samples:
  * owner: the innermost known pipeline step (driver stage or pass) on the stack;
  * inclusive: samples whose stack contains a function (each function once per sample);
  * self: leaf function, with allocator/hash-table frames attributed to their
    nearest kfrc caller;
  * with --focus, the callers and callees around one function.
Samples are proportions only: the counter is sampled, so absolute instruction
numbers come from the KFRC_PROFILE windows.
"""
import argparse
import collections
import gzip
import re

CRATE_HASH = re.compile(r'\[[0-9a-f]{16}\]')
GENERIC = re.compile(r'(?<=[\w])<[^<>]*>')
parser = argparse.ArgumentParser()
parser.add_argument('stacks')
parser.add_argument('--top', type=int, default=40)
parser.add_argument('--focus', action='append', default=[])
args = parser.parse_args()

# Innermost match wins (searched leaf to root).
STEPS = [
    'kfrc_son::verify::verify_graph', 'kfrc_son::verify::check',
    'kfrc_son::build::build_graph',
    'kfrc_son::transforms::ipo::identity::fold_identity_calls',
    'kfrc_son::transforms::ipo::plan::plan', 'kfrc_son::transforms::ipo::splice::splice',
    'kfrc_son::transforms::ipo::inline_calls',
    'kfrc_son::transforms::scalar::instcombine::run', 'kfrc_son::transforms::scalar::aggregate::run',
    'kfrc_son::transforms::scalar::gvn::run', 'kfrc_son::transforms::scalar::division::run',
    'kfrc_son::transforms::scalar::ranges::run', 'kfrc_son::transforms::scalar::seam::run_seeded',
    'kfrc_son::transforms::memory::forward::run', 'kfrc_son::transforms::memory::promotion::run',
    'kfrc_son::transforms::memory::observation::run', 'kfrc_son::transforms::memory::partition::run',
    'kfrc_son::transforms::control::conditions::run', 'kfrc_son::transforms::control::simplify::run',
    'kfrc_son::transforms::control::thread_variants::run', 'kfrc_son::transforms::control::variants::normalize',
    'kfrc_son::pipeline::partition_memory', 'kfrc_son::pipeline::optimize_graph',
    'kfrc_son::pipeline::optimize_specialized_seams', 'kfrc_son::pipeline::optimize_specialized_graph',
    'kfrc_son::lower_layout::lower_layout_with_options',
    'kfrc_son::specialize',
    'kfrc_x86::lower::private_slots::run', 'kfrc_x86::lower::schedule::Schedule>::new',
    'kfrc_x86::lower::branch::Branches>::new', 'kfrc_x86::lower::allocated::checker',
    'kfrc_x86::lower::allocated::allocation::Allocation>::new', 'kfrc_x86::lower::allocated::emit',
    'kfrc_x86::lower::allocated::constraints::select', 'kfrc_x86::lower::allocated::compile',
    'kfrc_x86::lower::Lowering>::run', 'kfrc_x86::lower::lower_function',
    'kfrc_x86::object_file', 'kfrc_x86::write_object',
    'rustc_codegen_kfrc::convert::convert_instance', 'rustc_codegen_kfrc::convert::definition',
    'rustc_codegen_kfrc::definitions', 'rustc_codegen_kfrc::CrateGraphs>::collect_references',
    'FunctionGraph as core::clone::Clone>::clone',
]
SUPPORT = ('malloc', 'free', 'realloc', 'calloc', 'tcache_bin_flush_small', 'malloc_default',
           'free_default', 'pa_alloc', '__rust_alloc', '__rust_dealloc', '__rust_realloc',
           'hashbrown::', 'alloc::raw_vec', 'core::ptr::drop_in_place', 'memcpy', 'memmove',
           'memset', '__memcpy', '__memmove', '__memset', 'RawVec', 'spec_from_iter', 'SpecFromIter',
           'SpecExtend', 'core::iter::', 'core::slice::sort', '[unknown]', 'Vec<', 'alloc::vec::')


def clean(frame):
    frame = CRATE_HASH.sub('', frame)
    return frame


def samples(path):
    frames = []
    with gzip.open(path, 'rt', errors='replace') as handle:
        for line in handle:
            line = line.strip()
            if not line:
                if frames:
                    yield frames
                frames = []
                continue
            parts = line.split(' ', 1)
            frames.append(clean(parts[1]) if len(parts) > 1 else '?')
    if frames:
        yield frames


def short(frame):
    previous = None
    while previous != frame:
        previous = frame
        frame = GENERIC.sub('', frame)
    return frame[:110]


def owner(frames):
    for frame in frames:
        for step in STEPS:
            if step in frame:
                return step.replace('kfrc_son::', 'son::').replace('kfrc_x86::', 'x86::')
    return '(other backend)'


def self_frame(frames):
    for frame in frames:
        if 'kfrc' in frame and not any(token in frame for token in SUPPORT[11:]):
            return short(frame)
    return short(frames[0])


total = 0
owners = collections.Counter()
inclusive = collections.Counter()
selfs = collections.Counter()
leaf_kind = collections.Counter()
focus_callers = collections.defaultdict(collections.Counter)
focus_callees = collections.defaultdict(collections.Counter)
for frames in samples(args.stacks):
    if not any('codegen_crate' in frame for frame in frames):
        continue
    total += 1
    owners[owner(frames)] += 1
    for name in {short(frame) for frame in frames}:
        inclusive[name] += 1
    selfs[self_frame(frames)] += 1
    leaf = frames[0]
    kind = ('allocator' if any(token in leaf for token in SUPPORT[:11]) else
            'hash table' if 'hashbrown' in leaf else 'other')
    leaf_kind[kind] += 1
    for focus in args.focus:
        for index, frame in enumerate(frames):
            if focus in frame:
                if index + 1 < len(frames):
                    focus_callers[focus][short(frames[index + 1])] += 1
                focus_callees[focus][short(frames[index - 1]) if index else '(self)'] += 1
                break


def table(title, counter, top):
    print(f'\n### {title}\n')
    print('| share | samples | function |')
    print('|---:|---:|---|')
    for name, count in counter.most_common(top):
        print(f'| {count / total:.2%} | {count} | `{name}` |')


print(f'samples inside codegen_crate: {total}')
print('leaf kinds: ' + ', '.join(f'{kind} {count / total:.1%}' for kind, count in leaf_kind.most_common()))
table('innermost pipeline step', owners, args.top)
table('self (allocator, hash-table and iterator frames charged to the nearest kfrc caller)', selfs, args.top)
table('inclusive', inclusive, args.top + 20)
for focus in args.focus:
    table(f'callers of {focus}', focus_callers[focus], 15)
    table(f'callees of {focus}', focus_callees[focus], 15)
