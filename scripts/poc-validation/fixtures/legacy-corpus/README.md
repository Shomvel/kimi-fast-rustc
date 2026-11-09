# Legacy scalar behavior corpus

This corpus carries selected executable behavior from the old root integration
suite into the real Rust compiler pipeline. It does not preserve the old
hand-written frontend or `son::Graph` API.

The seven suites contain **394 Rust functions, 1,295 parameter vectors, and 19
isolated trapping invocations**. Candidate07 passed all seven suites in the LLVM,
kfrc-opt0 and kfrc-opt1 arms. Its receipt is
`evidence/poc-semantic-only-20261109/results/candidate07/legacy-corpus/legacy-corpus/summary.json`;
later source changes require a new receipt.

The examples suite includes all 41 files from the pinned examples directory
(39 former `.kfr` programs, `hello.rs`, and `native_unit.rs`). The `hello` vector
also records its `kfrc-ok` output before the result line.

Each suite has:

- `lib.rs`: ordinary Rust. Each original program is isolated in its own public
  module. Only its `main` declaration becomes public `test_fn`; helper function
  calls still use the Rust ABI.
- `driver.rs`: built with LLVM for every arm. It uses `black_box` on inputs and
  calls the library. With no arguments it runs all returning cases. A trap case
  ID selects exactly one invocation in a separate process.
- `manifest.json`: original source, original test/file/line and revision, source
  SHA-256, actual fixture source, explicit typed input literals, expected signed
  64-bit result bits, and expected traps. The manifest contains the compiler
  flags needed for this contract.

`migration-status.json` inventories every selected old test individually. Its
scope is 54 files / 291 tests found in the pinned revision with old IR/frontend
references, including the mixed `short_jumps` file. The current disposition is
213 tests with selected runtime coverage, 23 concrete replacement-test mappings,
and 55 retired API/diagnostic tests. The evaluator fuel-limit assertion is
retained as a direct machine-level infinite-loop regression.
Thirty migrated runtime tests also retain separate machine-shape obligations.
The corrected read-only candidate07 object audit found 15 groups satisfied, 14
original shapes not met, and one Phi-temporary obligation requiring a virtual
instruction test. The original candidate07 audit falsely rejected five groups
because LLVM printed hexadecimal operands. The corrected audit normalizes
integer spelling for assertions and preserves raw instructions; the original
report remains available. Some exact-shape changes are
benign: the zero-return function is shorter and rustc merges early returns.
These distinctions are recorded per test; none are silently counted as covered. **These suites do not replace every old assertion.** A
migrated runtime test can still have a pending machine-shape assertion or an
unselected additional source variant; those are recorded on that test.

## Execution contract

Materialize and check the immutable inputs without invoking a compiler:

```sh
python3 scripts/poc-validation/legacy-corpus.py /tmp/legacy-inputs
python3 scripts/poc-validation/legacy-corpus.py --check
```

Use `legacy-corpus-run.py` for compilation and execution. It owns the LLVM,
kfrc-opt0, and kfrc-opt1 arms; this loader intentionally does not implement a
second runner. The driver must always be LLVM-built and linked against the
selected arm's rlib. Compile the libraries with the same rustc/toolchain,
`-Coverflow-checks=off`, panic abort, and the manifest's MIR settings.
`-Aarithmetic_overflow` permits intentionally wrapping runtime constant
expressions; const initializers still undergo CTFE checking.

Expected values preserve the old tests' `i64` bit-pattern convention: a `u64`
return of `u64::MAX` prints `-1`, bool prints 0/1, and unit prints 0 after actually
calling the function. Runtime add/subtract/multiply/negation and shifts retain
the old wrapping/masked-count semantics through overflow checks off. This flag
does **not** disable signed `MIN / -1`, signed `MIN % -1`, or zero-divisor traps.
Those are separate processes: success is forbidden; a nonzero trap is required;
timeout is failure. A panic abort and a hardware fault need not have the same
platform-specific numeric exit status. Match guards, short circuit operators,
and zero-trip loops include untaken division paths to ensure traps are not
speculated.

Constant initializers remain Rust CTFE, with their original successful results.
No custom evaluator or parser supplies the reference answer. Original explicit
expected tables are copied; formula-based legacy boundary tables are expanded
into fixed vectors. Every successful arm must match those expected outputs,
not merely match another potentially incorrect compiler arm.

## Deliberately distinct obligations

- Parser restrictions and exact diagnostic text are retired with the old
  frontend. Valid Rust such as CTFE calls, overlapping match patterns, or
  additional visibility syntax must not be rejected just to imitate that
  subset. rustc owns syntax, typing, reachability diagnostics, and CTFE failures.
- Old `usize == u64` / `isize == i64` type aliases, uppercase radix spellings,
  const-name shadowing, and malformed/non-Rust source cannot be mechanically
  copied as equivalent Rust. Such cases remain explicit in the inventory
  unless given a documented adaptation.
- Exact x86 bytes, register allocation, spills, flags, immediate selection,
  branch relaxation, and instruction counts require machine-level tests. A
  correct returned number alone does not prove these properties.
- The old Graph cache keys, function extraction, parallel worker error ordering,
  source-batch CLI, and node-ID/fingerprint timing benchmarks describe removed
  APIs. They do not measure the semantic IR pipeline and do not justify speedup
  claims.
- Hand-built old Graph safety regressions are **not** automatically obsolete:
  poison, dominance, Phi snapshots, effects, and trap ordering require a new
  semantic graph or real-Rust case. Unmapped cases stay pending.

Compilation logs, execution receipts, and timing measurements belong in the
validation output directory, not in these source fixtures.

## Machine-shape audit

Inspect a completed real-Rust corpus run without compiling or executing again:

```sh
python3 scripts/poc-validation/legacy-machine-shapes.py \
  --run /path/to/legacy-corpus-run-output \
  --out /path/to/new-machine-audit \
  --objdump llvm-objdump
```

The audit checks each rlib SHA against the original runner receipt, uses its
embedded source manifest to identify functions, and bounds instructions by ELF
symbol sizes. It preserves the old immediate, shift, division, flag, spill, and
byte assertions. Decimal and hexadecimal operand spellings are equivalent for
assertions; the report retains raw operands and the audit script SHA.
`--require-all` makes any gap or missing measurement a failure;
without it the command produces a diagnostic report. Runtime values and traps
remain the responsibility of the three-arm corpus runner.
