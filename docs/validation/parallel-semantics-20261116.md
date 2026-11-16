# Parallel compilation, Rust semantics and LLVM comparison

The accepted implementation is `8d482bb`; its complete Git tree is
`2b5a09228fb59f3f5cc48cde4355c3803b6201b4`. The delivery adds documentation and
connects the takeover history to PR #8 without changing that implementation.

## Delivered scope

This PR includes the earlier arithmetic, semantic inlining and parallel graph
construction work, followed by the Cranelift machine layer and the accepted
2026-11-16 refinements. Rust semantic optimization remains in R/S/L graphs;
Cranelift performs machine lowering, allocation and encoding. The original x86
machine layer remains a differential oracle. Unsupported inputs still fail
explicitly, without a function-level LLVM fallback.

The Cranelift and ELF interfaces are documented in the
[machine-layer contract](../design/machine/layer.md),
[representation contract](../design/ir/representation.md) and
[program-data contract](../design/ir/program-data.md).
[Cranelift patch provenance](../../third_party/cranelift-codegen/KFRC-PATCHES.md)
records the local changes and upstream version/license. The allocator is the
vendored `regalloc2 0.15.2`, with its upstream license retained.

| Accepted commit | Behavior |
|---|---|
| `877f9c8` | Use nonconvex known-bit facts for scoped comparison implication. |
| `fe14a88` | Bound validated reference-slice metadata by allocation extent; keep raw pointers and ZSTs outside that proof. |
| `984d19e` | Split cold fixed-register call arguments with correctly typed temporary copies. |
| `80b41a8` | Perform full variant-threading cleanup after the worklist drains. |
| `c09fcf1` | Admit late Cargo jobserver tokens while the calling thread compiles; retain ordered output and panic cleanup. |
| `354f378` | Fold effect-free diamonds through empty control wrappers. |
| `a680593` | Fuse ordered bounds and compatible one-bit interval unions after layout. |
| `57c45e9` | Pack consecutive constant stack-byte writes into exact-width stores. |
| `ea10f16` | Build threading dominance only when a viable predecessor decision needs it. |
| `b37a98d` | Remove constant fallback diamonds using a separately proved wrapping-value expression. |
| `e45f2c7` | Materialize explicitly extended constant returns directly in their ABI form. |
| `4370ec3` | Consume owned graphs during final layout, avoiding the transactional API's clone. |
| `2b2e925` | Retain compact operand-slot encodings in immutable use entries. |
| `84c84b4` | Avoid visited-set insertion at terminal condition predicates. |
| `6a40bec` | Display integer source contracts in graph dumps. |
| `8ea077e` | Preserve producer scalar widths and ABI extension contracts. |
| `8559b55` | Encode unextended I8/I16 constant returns using only the required value bits. |
| `8d482bb` | Compute detailed SCC report data only when observed. |

The scalar ABI change fixes a real `repr(transparent)` signed-byte parameter
whose sign-extension contract was previously lost. Bool retains source `i1`
and storage `i8`. Unextended constant returns use independent return virtual
registers so a shared store or a second return lane keeps its own contract.
Explicit UEXT/SEXT still defines the complete required register value; zero
keeps the existing xor path.

Graph-buffer reuse, Uses cursors and CGSCC seam cleanup were measured and
withdrawn. The SCC trace probe was diagnostic only and is excluded from this
implementation. SCC preparation still uses serial global Work-budget and
readiness decisions; speculative or parallel SCC preparation is unfinished.

## Correctness evidence

Validation used Linux x86-64, Rust 1.94.1 / LLVM 21.1.8 and an isolated
8-vCPU / 16-GiB environment. Native differential fixtures use common sources
and ABI providers. The O3 comparisons disable MIR optimization consistently.

- The complete gate at `8559b55` passed **1,427 workspace tests, 1 ignored**,
  including 928 SoN and 21 Cranelift tests; 30 shell tests and every original
  fixture/E2E command also passed. `RUST_TEST_THREADS=1` isolates the previously
  observed ETXTBSY fixture condition; Cargo jobs and compiler concurrency remain
  independent settings.
- The six-arm scalar ABI matrix passed **2,333,464 record assertions per arm**,
  checksum `655e976d671ceeed`. LLVM, candidate 1/4-thread and original x86
  paths each passed all 256 strict signed-wrapper assertions, checksum
  `0408102040810202`. The old producer's separately preserved SIGILL
  counterexample is a recorded failure, not a passing oracle or a repeated test.
- Four original library corpora, all 13 workload checksums, 514 drop cases,
  257 slice lengths and ZST length `usize::MAX` agreed with LLVM. Candidate
  1/4-thread library objects agreed exactly.
- At `8d482bb`, all 16 existing SCC tests passed, and the release backend built.
  Default 1/4-thread objects and all ten native/semantic final drivers exactly
  matched the accepted predecessor. Separate enabled-stats compiles matched
  all 1,517 ordered rows, four summaries and complete stderr byte for byte.
  The unchanged full gate and ABI matrix were not rerun for this two-file
  reporting-only change.

Committed reproduction entry points, in the pinned Linux environment:

```sh
RUST_TEST_THREADS=1 cargo test --workspace
RUSTC_BOOTSTRAP=1 cargo build --release --manifest-path crates/rustc_codegen_kfrc/Cargo.toml
RUSTC_BOOTSTRAP=1 cargo test --manifest-path crates/rustc_codegen_kfrc/Cargo.toml --lib codegen::cgscc
export KFRC_CODEGEN_BACKEND="$PWD/crates/rustc_codegen_kfrc/target/release/librustc_codegen_kfrc.so"
bash crates/rustc_codegen_kfrc/tests/scalar-abi/run.sh "$KFRC_CODEGEN_BACKEND" /tmp/kfrc-scalar-abi
bash crates/rustc_codegen_kfrc/tests/semantic-ir/run.sh "$KFRC_CODEGEN_BACKEND" /tmp/kfrc-semantic
```

The output directories must be fresh. These entry points cover the workspace
and focused regressions; they are not a claim that this short command list
reproduces the entire archived gate or its formal performance batches.

## Measured results and limits

Formal compile measurements discard one warmup and serially interleave three
repeats on the same environment. Each batch checks 80 compiled objects against
its frozen correctness references and audits every perf event for valid counts
and full coverage. RSS is reported per crate, never summed. Profile collection,
builds and transfers run outside formal measurements. Percentages below belong
to their own immediate-predecessor batches and must not be compounded.

| Change | Observation | Interpretation |
|---|---|---|
| Producer scalar ABI | Four-library text 75,130 → 74,742 B; compile instructions +0.1113% / +0.1464% at 1/8 threads; 13-workload runtime instructions geomean −0.0910%. | Correctness fix with small costs; base64 decode/short instructions increase 0.4200% / 0.1483%. |
| Unextended narrow returns | I8 6 → 3 B, I16 6 → 5 B; shared store/return I8 9 → 6 B, I16 11 → 10 B, matching actual LLVM encodings. Compile instructions +0.0086% / +0.0126%. | Fixture size benefit; four real-library objects and 12 final drivers are unchanged from the ABI predecessor. |
| Narrow-return microbenchmark | One common LLVM caller, three frozen libraries, nine fixed 100-million-call workloads; 81 preparation checks and 108 formal/warmup event audits pass. Cycles geomean +0.0338%, instructions effectively identical. | No runtime acceleration or meaningful observed regression; kept separate from the 13 real workloads. |
| SCC report accounting | Compile instructions −0.0505% / −0.0620%; wall −1.4362% / +0.1771%; RSS four medians rise and four fall. | Small work reduction; no stable wall-time or memory improvement claim. Objects and diagnostic output are unchanged. |

The final SCC-report batch records the following sums of per-crate medians;
they are four captured crate commands, not an end-to-end Cargo project build:

| Arm | Instructions, billions | Wall, ms |
|---|---:|---:|
| Narrow predecessor, 1 thread | 9.044628 | 1503.126 |
| Accepted, 1 thread | 9.040058 | 1481.538 |
| Narrow predecessor, 8 threads | 9.073664 | 1280.362 |
| Accepted, 8 threads | 9.068043 | 1282.630 |
| LLVM MIR0 | 7.131171 | 1084.068 |

Earlier real-Cargo jobserver validation confirmed that `-j1` starts no extra
codegen worker, and that workers admitted at `-j4` hold real inherited tokens.
These checks validate scheduling, not equivalent gains in a saturated multi-crate
build. The independent SCC trace established budget/readiness invariants; it
did not execute speculation, replay or parallel preparation. LLVM parity and
roughly 50% compile acceleration remain unachieved goals.

## Provenance and archive availability

[Structured acceptance data](parallel-semantics-20261116.json) records backend
and archive SHA256 identities. The archives contain raw commands, outputs,
source/toolchain/input pins, native artifacts and event audits. They have been
downloaded, hash-verified and retained outside Git; this repository publishes
their identities and summarized results, not a public download location.

The takeover root `cbd4858` is a reconstructed snapshot. Delivery merge
`9fd1974` connects it to the previous PR #8 tip `7fb99c5`, which already
contains main `b254941`. The merge preserves the accepted `8d482bb` tree
exactly, retains both histories, and permits a normal fast-forward PR-branch
update without rewriting upstream history. Only the delivery documentation
changes after that accepted implementation.
