#!/usr/bin/env python3
"""Codemod: route kfrc's std HashMap/HashSet through the Fx hasher.

usage: python3 -I tools/compile-cost/fx_hash_codemod.py <repository root> [--check]

Rewrites every `.rs` file under the kfrc-body, kfrc-son, kfrc-x86 and
rustc_codegen_kfrc `src/` trees and the first three crates' integration `tests/`
(they build public tables such as `SpecializeContext::overrides`); the
rustc_codegen_kfrc `tests/` tree (rustc-upstream vendor sources) is skipped:

* `use std::collections::{..., HashMap, HashSet, ...};` keeps the other names on
  std and imports HashMap/HashSet from `kfrc_body::fx_hash` (`crate::fx_hash`
  inside kfrc-body);
* qualified `std::collections::HashMap`/`HashSet` paths take the same module;
* constructors (also through file-local `type X = HashMap<..>` aliases) that only exist for `RandomState` become their hasher-generic
  forms: `new()` -> `default()`, `with_capacity(n)` ->
  `with_capacity_and_hasher(n, Default::default())`, `from([..])` -> `from_iter([..])`.

A file that was rustfmt-clean before the rewrite is passed through `rustfmt
--edition 2024` afterwards (requires rustfmt on PATH); other files keep their
layout. The rewrite is idempotent, so the merge parent can re-run it after merging other
streams and get the same result on files that were already converted. `--check`
prints the files that would change and exits 1 when any would.
Requires `crates/kfrc-body/src/fx_hash.rs` (committed separately).
"""
import re
import subprocess
import sys
from pathlib import Path

CRATES = ('kfrc-body', 'kfrc-son', 'kfrc-x86', 'rustc_codegen_kfrc')
HASHED = ('HashMap', 'HashSet')
USE_GROUP = re.compile(r'use std::collections::\{([^}]*)\};')
USE_SINGLE = re.compile(r'use std::collections::(HashMap|HashSet);')
QUALIFIED = re.compile(r'std::collections::(HashMap|HashSet)\b')
# Constructors tied to RandomState. `(?<![\w:])` keeps `FxHashMap::new` style
# names and already-qualified paths of other modules untouched.
NEW = re.compile(r'(?<![\w])((?:[\w:]*::)?(?:HashMap|HashSet)(?:::<[^;{}]*?>)?)::new\(\)')
WITH_CAPACITY = re.compile(r'(?<![\w])((?:[\w:]*::)?(?:HashMap|HashSet)(?:::<[^;{}]*?>)?)::with_capacity\(')
FROM = re.compile(r'(?<![\w])((?:[\w:]*::)?(?:HashMap|HashSet))::from\(')
ALIAS = re.compile(r'\btype (\w+)\s*=\s*(?:[\w:]*::)?(?:HashMap|HashSet)<')


def module_path(crate, tree):
    return 'crate::fx_hash' if (crate, tree) == ('kfrc-body', 'src') else 'kfrc_body::fx_hash'


def balanced_call_end(text, open_index):
    depth = 0
    for index in range(open_index, len(text)):
        if text[index] == '(':
            depth += 1
        elif text[index] == ')':
            depth -= 1
            if depth == 0:
                return index
    raise ValueError('unbalanced call')


def rewrite_with_capacity(text):
    out = []
    position = 0
    for match in WITH_CAPACITY.finditer(text):
        if match.start() < position:
            continue
        open_index = match.end() - 1
        close_index = balanced_call_end(text, open_index)
        argument = text[open_index + 1:close_index]
        out.append(text[position:match.start()])
        out.append(f'{match.group(1)}::with_capacity_and_hasher({argument}, Default::default())')
        position = close_index + 1
    out.append(text[position:])
    return ''.join(out)


def rewrite(text, module):

    def group(match):
        names = [name.strip() for name in match.group(1).split(',') if name.strip()]
        hashed = [name for name in names if name in HASHED]
        if not hashed:
            return match.group(0)
        rest = [name for name in names if name not in HASHED]
        # `crate::`/`kfrc_body::` sort before `std::`, as rustfmt orders a block.
        lines = [f'use {module}::{{{", ".join(hashed)}}};' if len(hashed) > 1
                 else f'use {module}::{hashed[0]};']
        if rest:
            lines.append('use std::collections::{' + ', '.join(rest) + '};' if len(rest) > 1
                         else f'use std::collections::{rest[0]};')
        return '\n'.join(lines)

    text = USE_GROUP.sub(group, text)
    text = USE_SINGLE.sub(lambda match: f'use {module}::{match.group(1)};', text)
    text = QUALIFIED.sub(lambda match: f'{module}::{match.group(1)}', text)
    text = NEW.sub(lambda match: f'{match.group(1)}::default()', text)
    # Local aliases of the tables (`type State = HashMap<..>`) lose `new()` too.
    for alias in ALIAS.findall(text):
        text = re.sub(rf'(?<![\w:]){alias}::new\(\)', f'{alias}::default()', text)
    text = rewrite_with_capacity(text)
    text = FROM.sub(lambda match: f'{match.group(1)}::from_iter(', text)
    return text


def formatted(text):
    """rustfmt (edition 2024) output for one file's text, or None if rustfmt fails."""
    result = subprocess.run(['rustfmt', '--edition', '2024'], input=text, capture_output=True,
                            text=True)
    return result.stdout if result.returncode == 0 else None


def main():
    root = Path(sys.argv[1])
    check = '--check' in sys.argv[2:]
    assert (root / 'crates/kfrc-body/src/fx_hash.rs').is_file(), 'commit fx_hash.rs first'
    changed = []
    for crate in CRATES:
        trees = ('src',) if crate == 'rustc_codegen_kfrc' else ('src', 'tests')
        for tree in trees:
            for path in sorted((root / 'crates' / crate / tree).rglob('*.rs')):
                if path.name == 'fx_hash.rs':
                    continue
                original = path.read_text()
                updated = rewrite(original, module_path(crate, tree))
                if updated != original and formatted(original) == original:
                    # Keep rustfmt-clean files clean (import order in a block).
                    updated = formatted(updated)
                if updated != original:
                    changed.append(path.relative_to(root))
                    if not check:
                        path.write_text(updated)
    library = root / 'crates/kfrc-body/src/lib.rs'
    text = library.read_text()
    if 'pub mod fx_hash;' not in text:
        changed.append(library.relative_to(root))
        if not check:
            library.write_text(text.replace('pub mod body;\n', 'pub mod body;\npub mod fx_hash;\n', 1))
    for path in changed:
        print(path)
    print(f'{len(changed)} files {"would change" if check else "rewritten"}', file=sys.stderr)
    if check and changed:
        sys.exit(1)


main()
