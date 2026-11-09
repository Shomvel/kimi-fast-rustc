# Pinned upstream Rust compiler tests

This corpus contains byte-exact test sources from Rust **1.94.1**, commit
`e408947bfd200af42db322daf0fadfe7e26d3bd1`. The archive is
<https://codeload.github.com/rust-lang/rust/tar.gz/e408947bfd200af42db322daf0fadfe7e26d3bd1>,
SHA-256 `063f7b364a2f0dee6d8f6733486ea7b6510c79682129448b2e6bd4ffc3e2a8fe`.
Every primary source, sidecar, dependency and auxiliary file has an original URL
and SHA-256 in `manifest.json`. The sources and auxiliary paths retain the
upstream tree below `vendor/tests`; no Rust source is patched or extracted into
a replacement harness. Original `COPYRIGHT`, `LICENSE-APACHE` and `LICENSE-MIT`
are retained in `vendor/` and checksummed in the manifest.

The declared candidate pool is **427** explicit `run-pass`, `build-pass` or
`run-fail` primary `.rs` files in the UI directories listed under
`selection_policy.directories_by_family`. It covers integer operations,
control flow, references, aggregates, enums, function calls, moves/drop and
panic behavior. Auxiliary sources are dependencies, not additional tests.
Other directories, UI compile-fail/check-only tests, codegen tests, run-make,
documentation tests and library suites are outside this first pool.

The first batch originally selected **160** candidates. Within each family the importer
prefers assertion-bearing programs, then shorter files, then lexical upstream
paths. The fixed quotas are integer 25, control-flow 25, reference 25,
aggregate 20, enum 20, call 10, drop 30, panic 5. The original **73** blocked
and **194** deferred candidates remain in the same manifest with reasons and
their original source. A blocked test can have several reasons. Capability
checks conservatively identify floating point, integers wider than 64 bits,
unwind, atomics, SIMD, assembly, threads/TLS, asynchronous code, custom runtimes,
foreign ABIs and external inputs. This is a visible initial scope decision,
not evidence that every excluded construct is unsupported by the backend.

Classifier version 2 corrects one original member:
`ui/binop/compound-assign-by-ref.rs` executes `3f32` and `6f64` despite having
no bare `f32`/`f64` type names or decimal points. Decimal integer/exponent
literals with float suffixes, including underscore separators, now require
float support. Radix integer tokens such as `0xff32` and `0xdead_beef_f64`
remain integer literals. Version 2 produced **159 selected,
74 blocked, 194 deferred**, still covering the same 427 candidates. No vacant
quota is refilled. `initial_selection`, `selection_batch.original_selected_ids`
and its fingerprint preserve all 160 original members; `classification_updates`
records the affected source lines and status change. All upstream file bytes,
hashes and candidate identities are unchanged.

Classifier version 3 also recognizes `i128`/`u128` suffixes on decimal,
hexadecimal, octal and binary literals, including underscores and macro
arguments. Bare type names were already covered. The complete pool scan finds
two additional affected candidates: `next-power-of-two-overflow-ndebug.rs`
executes `1u128` at line 13 and changes from selected to blocked;
`i128-min-literal-parses.rs` has a hexadecimal `i128` literal at line 4 and
changes from deferred to blocked. The current planning counts are therefore
**158 selected, 76 blocked, 193 deferred**. No replacement test is selected.
The version-2 float delta remains intact, and version-3 entries append their
previous selections and source evidence. `classifier_history` retains the
earlier manifest fingerprints and planning counts.

Historical batch results keep their original manifest fingerprint and counts.
In particular, the initial 151-ready plan and its recorded checkpoint of
22 passing, 5 kfrc failures and 2 LLVM reference failures are not rewritten by
this correction. A future execution plan must calculate its protocol gates
from the corrected manifest and store a new fingerprint.
The next recorded checkpoint, batch 02 with 61 passing candidates, 62 passing
revisions, 6 newly observed failures and 78 deferred candidates, likewise keeps
its original report and manifest identity.

The official harness's global `tests/auxiliary/minicore.rs` support file is also
vendored and hashed. It is required by compiletest and is not a test case.
The pinned UI harness also scans `src/etc/natvis`, `src/tools/run-make-support`
and `src/tools/compiletest` while constructing its common-input stamp. Those
complete upstream directories and the seven GDB/LLDB pretty-printer inputs
from `src/etc` are preserved under `vendor/src`. The manifest's separate
`support_files` ledger records all 98 original resources with URLs and hashes,
relative to the manifest directory; they are not test cases. This resource set
comes from the pinned `common_inputs_stamp`, including its unconditional
directory scans and optional file stamp inputs.

`selection.status` is a planning decision. It is never a pass/fail result.
The execution adapter must interpret all original directives, target gates,
revision conditions, auxiliary builds and runtime expectations before running
each revision. Unknown or unsupported protocol requirements must be reported
explicitly. Runner results belong in a separate output directory and must
account for selected, blocked and excluded candidates. An unexecuted or blocked
candidate must not disappear into a passing denominator.

Upstream `run-fail` is not simply a requirement for any nonzero exit: the pinned
official UI harness distinguishes normal failure from a crash using the target's
`can_unwind` fact. A `panic=abort` execution therefore does not automatically
satisfy an upstream panic test on an unwinding Linux target. Original directives
and expectations remain intact; see the official compiletest adapter under
`scripts/rustc-upstream` for execution details.

To reproduce the import into a new directory, or verify the checked-in corpus:

```sh
python3 scripts/rustc-upstream/import_tests.py \
  --archive /tmp/rust-e408947bfd200af42db322daf0fadfe7e26d3bd1.tar.gz \
  --output /tmp/new-rustc-test-corpus

python3 scripts/rustc-upstream/import_tests.py \
  --archive /tmp/rust-e408947bfd200af42db322daf0fadfe7e26d3bd1.tar.gz \
  --verify
```

The importer downloads the pinned archive if absent, verifies its hash before
reading sources, and refuses to overwrite an existing manifest. `--verify`
compares every vendored byte, file hash, test hash, license and directive ledger
against that archive. Import and verification do not compile Rust code.
