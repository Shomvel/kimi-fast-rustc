# Validation evidence

## Environment

Formal validation uses `yicloud-dev`, Linux x86_64, four available CPUs, and the official Rust 1.99.1 toolchain. Benchmark inputs are ripgrep 15.2.0 (`e89fff89ac9af12e8d4ce9d5fd07beb408ca730f`) and cargo crate 0.99.0 / CLI 1.98.0 (`797e8a9bca276c1c9f9f738d2a20f484fa4eea9d`). Dependencies are fetched before timed builds and used offline.

## Confirmed native correctness

On `yicloud-dev`, the project test suite and Clippy passed: 322 tests, zero failures. The expanded final source passed 381 tests and Clippy with Rust 1.99.1. A separate fresh GitHub clone of the initial `56ae36c31a13513e82fa1209b88cf71d488b4b19` checkpoint also passed all 322 tests and Clippy with Rust 1.99.1. The native executable path compiled `examples/native_main.kfr` through sea-of-nodes optimization, linear-scan allocation, GNU assembly, and system `as`/`gcc`; the resulting x86_64 executable returned 42.

The direct object path generated an ELF64 little-endian x86_64 relocatable object without an assembler. `readelf` confirmed a global `main` function symbol. A restricted-PATH run had no `as` available and still produced the object. Linking that object with GCC produced an executable that returned 42.

A separate multi-function object used an `E8 rel32` call and an `R_X86_64_PLT32` relocation with addend `-4` from `main` to `native_answer`. `readelf -W -r` verified the relocation, and the linked x86_64 executable returned 42 on `yicloud-dev`.

SoN/reference differential tests cover arithmetic, comparisons, wrapping boundaries, register and stack parameters, high register pressure, and spills. These tests do not replace the real x86_64 execution above.

## Parallelism and caching evidence

A 64-function, 512-operation-per-function workload on the 12-core Apple M3 Pro compared DAG and three-stage pipeline compilation over 12 interleaved samples. Median DAG times were 104.861 ms at one job and 47.969 ms at four jobs; pipeline times were 48.392 ms and 27.556 ms. Every one of the 3,328 objects, including warmups, was byte-identical across modes. These figures are specific to that synthetic workload and host.

A separate 385-binding object-compilation measurement gave medians of 4.380 ms without cache, 9.275 ms for a cold cache, and 4.189 ms for a hot cache. All outputs were byte-identical. Cold-cache hashing currently costs more than it saves on this small input; hot-cache reuse is only modestly faster and must not be oversold.

The audit directory path was independently rechecked with a real proc-macro and a dependent binary. Separate JSONL records showed no `lto` for the proc-macro and `lto=thin` plus `embed-bitcode=yes` for the final binary. Explicit single-file audit output also worked without `Is a directory` errors.

The real PGO probe completed instrumentation, two training runs, nonempty profile merging, profile-use compilation, and final execution on both macOS and `yicloud-dev`, using matching `llvm-profdata` from Rust 1.99.1.

## Whole-project build prechecks

Both pinned projects built successfully offline with stock Rust 1.99.1:

- ripgrep 15.2.0 produced `rg` and reported its expected version.
- cargo 0.99.0 produced `cargo` and reported its expected version.

These are build prechecks, not performance results.

## Formal benchmark status

Early attempts stopped before compilation because the fetch target and `/usr/bin/time` were unavailable. Those issues were fixed. A later run built all three ripgrep arms and Cargo's baseline/controlled arms, then stopped when ordinary LTO was rejected for a proc-macro crate. The proc-macro-specific filter was subsequently fixed without disabling LTO for final binaries.

The successful ripgrep artifacts from `/root/kimi-fast-rustc-benchmarks/results/20261107T154033Z` were measured separately with 15 alternating samples:

| Arm | Binary bytes | Median runtime | Size ratio | Runtime ratio |
|---|---:|---:|---:|---:|
| baseline | 30,571,200 | 4.417 ms | 1.000 | 1.000 |
| controlled | 30,571,200 | 4.682 ms | 1.000 | 1.060 |
| candidate | 3,845,896 | 4.350 ms | 0.126 | 0.985 |

Baseline and controlled binaries have the same SHA-256, confirming exact controlled code generation; their runtime difference is sampling noise from short processes. The candidate is about 87.4% smaller and its median runtime is about 1.5% faster in this partial measurement. Cold compile wall times were 14.55 s baseline, 14.80 s controlled, and 28.57 s candidate, so the current ThinLTO candidate trades compile time for artifact size.

A larger 30-sample search over the Cargo source tree gave median runtimes of 11.157 ms baseline, 11.158 ms controlled, and 11.203 ms candidate. The controlled ratio was 1.00007 and the candidate ratio 1.00415. This larger workload is approximately at parity but does not meet the strict `candidate / baseline <= 1.0` rule, so it is not presented as a confirmed runtime win.

The earlier measurements are valid partial LLVM reference-track results; the larger workload did not establish a runtime win.

## Completed formal benchmark

The complete three-arm run finished successfully in `/root/kimi-fast-rustc-benchmarks/results/20261107T155940Z`, using Rust 1.99.1, four build jobs, offline dependencies, and 15 alternating runtime samples per arm:

| Project | Arm | Binary bytes | Median runtime | Compile wall | Size ratio | Runtime ratio |
|---|---|---:|---:|---:|---:|---:|
| ripgrep | baseline | 30,571,200 | 4.640 ms | 15.23 s | 1.000 | 1.000 |
| ripgrep | controlled | 30,571,200 | 4.673 ms | 15.28 s | 1.000 | 1.007 |
| ripgrep | candidate | 3,845,896 | 4.322 ms | 28.63 s | 0.126 | 0.931 |
| cargo | baseline | 40,321,488 | 48.198 ms | 140.91 s | 1.000 | 1.000 |
| cargo | controlled | 40,321,488 | 48.073 ms | 140.81 s | 1.000 | 0.997 |
| cargo | candidate | 23,515,320 | 47.634 ms | 323.36 s | 0.583 | 0.988 |

Both candidate size and median runtime ratios are at or below 1.0 for both projects. The ripgrep candidate is about 87.42% smaller with a 6.87% shorter median runtime; the cargo candidate is about 41.68% smaller with a 1.17% shorter median runtime. The candidate's ThinLTO builds are slower than stock builds, while controlled compile time is effectively unchanged. The ripgrep controlled runtime ratio of 1.007 is short-process sampling variation, not a code-generation difference.

These results satisfy the requested artifact-size and runtime parity criterion for the LLVM reference track on the recorded host and workloads. They do not imply that the experimental native backend can compile complete ripgrep or cargo workspaces.

## Follow-up optimization evidence

A separate locked cold-build sweep compared one, two, and four Cargo jobs. Ripgrep took 54.210/28.785/15.353 seconds, while cargo took 497.406/280.870/142.251 seconds. Four jobs gave 3.53x and 3.50x speedups over one job with identical output hashes; maximum single-process RSS increased about 0.9% for ripgrep and 12.3% for cargo. Controlled-wrapper overhead was approximately 0.8–1.0% in this comparison.

ELF analysis found that most ripgrep size reduction came from removing DWARF and static symbols, while cargo's remaining file content was 77.76% `.text`. Ripgrep's remaining file content was 61.64% `.text`. Candidate load-segment alignment left only tens to hundreds of unused bytes, so further alignment tuning is not a meaningful optimization target. Code generation and LTO experiments are the useful next step.

A CGU sweep produced the following cold-build data; runtime sampling is still pending and these partial data must not be used to claim parity:

| Project | CGU | Compile wall | Binary bytes |
|---|---:|---:|---:|
| ripgrep | 1 | 28.117 s | 3,845,896 |
| ripgrep | 4 | 23.494 s | 4,211,216 |
| ripgrep | 8 | 24.032 s | 4,122,992 |

CGU4 reduced compile time by about 16.4% but increased binary size by about 9.5%; CGU8 reduced compile time by about 14.5% and increased size by about 7.2%.

Cargo showed a larger compile-time tradeoff: CGU1 took 327.366 seconds and produced 23,515,320 bytes; CGU4 took 200.736 seconds and produced 25,071,256 bytes; CGU8 took 234.017 seconds and produced 25,810,488 bytes. Thus CGU4 reduced compile time by 38.7% with 6.6% larger output, while CGU8 reduced compile time by 28.5% with 9.8% larger output. Maximum single-process RSS was 2,211,808/3,287,892/2,496,428 KiB respectively. Runtime sampling is still pending.

## Native-only control flow and calls

The active architecture is now native-only; the LLVM experiments above are historical evidence and are not a supported fallback.

`if` expressions are lowered to `If`, control projections, regions, and Phi nodes, then to x86 labels and conditional jumps. Nested `if` tests pass on both the reference evaluator and real x86_64 execution. A real `if 2 > 1` executable returned 42 on `yicloud-dev`. Standard `fn main() {}` and `return;` produce exit status zero.

Function calls preserve function metadata, use separate machine functions, save caller-saved registers, and emit `R_X86_64_PLT32` relocations. Real `main -> answer` and `id(42)` calls returned 42 on `yicloud-dev`, and `readelf -W -r` verified their relocations. Calls use the first six System V argument registers and outgoing stack arguments after all source values are preserved. `sum8(1..8)` returned 36 on `yicloud-dev`; its seventh and eighth arguments were written with `[rsp]` and `[rsp+8]` before the call. Unsupported signatures and arity mismatches are rejected explicitly.

Mutable `let` bindings and assignments use SSA versioning. `while` loops use header Phis and explicit `Jump` back edges. Loop-aware live-range extension prevents condition inputs from being clobbered before later iterations; temporary Phi copies preserve back-edge values. Reference tests cover zero and multiple iterations, nested loops, sequential assignments, body-local bindings, spills, and calls. Real x86_64 execution on `yicloud-dev` returned 55 for `sum_to(10)`, 4 for the nested-loop example, and preserved the earlier 42/36/0 results for `if`, `sum8`, and standard `main`.

Statement-level `if` supports optional `else`, `else if` chains, branch-local bindings, and mutable-variable Phis. Reachable arms terminate in explicit jumps to the merge region; this also handles an inner loop inside a branch without falling through into the other arm. Seven integration tests cover branch outcomes, missing `else`, nested statements and `else if` chains, loops containing conditions, branches containing loops, and rejected scope/terminal-statement cases. The real statement-`if` example returned 42 on `yicloud-dev`, while loop and expression-`if` regressions retained their earlier results.

Mutable parameters participate in assignment and loop Phi versioning. The five compound operators `+=`, `-=`, `*=`, `/=`, and `%=` read the previous SSA value and emit the corresponding binary operation. Tests cover every operator, loops, calls, immutable-target rejection, and division traps. The real `else if` example combines a mutable parameter, compound assignments, nested branches, and a cross-function call; it returned 42 on `yicloud-dev`.

The integer backend now supports wrapping variable negation, SoN bitwise inversion, `&`, `|`, `^`, `<<`, and arithmetic `>>`. The frontend implements shift/AND/XOR/OR precedence. Six integration tests compare the reference and machine evaluators on precedence, signed values, count masking, `i64` boundaries, spills, and loop Phis. Shift lowering protects the left input before loading `cl`, covering values allocated to `rcx`. On `yicloud-dev`, the direct ELF object for the bitwise example linked and returned 42.

The frontend now distinguishes `i64` from `bool` in bindings, expressions, parameters, returns, and calls. Comparisons produce `bool`, conditions reject integers, and `if` expressions require matching arm types. Logical `!` compares against zero; `&&` and `||` use conditional regions so the right operand executes only when required. Seven integration tests cover truth tables, precedence, mutable loop-carried booleans, function signatures, type mismatches, and skipped division-by-zero on short-circuit paths. The real cross-function boolean example returned 42 on `yicloud-dev`; bitwise, `else if`, and loop regressions retained their results.

Loop headers now accept ordinary back edges and additional `continue` edges. `break` captures current SSA values and merges them with normal condition exits; terminal `if` arms do not emit fake merge jumps. Eight integration tests cover exit values, selected iterations, one or two terminal arms, nested loops, typed exit Phis, invalid control statements, and an actual infinite loop. The reference evaluator stops after one million instructions rather than hanging on nontermination. The real `break`/`continue` example returned 42 on `yicloud-dev`, with boolean and ordinary-loop regressions unchanged.

Early `return` is supported inside conditionals and loops. If every path returns, no trailing return is required; otherwise a typed function must return the declared type on its reachable tail. The backend emits every return and its callee-saved restoration. Single-function lowering rejects multiple `Start` nodes, while `native_program` splits each function at its final return. Six integration tests cover early if/loop returns, two returning arms, unit and bool returns, cross-function calls, and reachability/type errors. The real early-return example returned 42 on `yicloud-dev`.

The scalar type system now includes `u64` alongside `i64` and `bool`. Decimal `u64` literals cover the full bit range, and unsuffixed literals can be inferred in a `u64` context. SoN distinguishes unsigned comparisons, `UDiv`, `URem`, and logical `LShr`; constant folding, value numbering, and cache keys preserve that distinction. Unsigned division traps only on zero, not signed overflow. Six integration tests cover inference, ordering, arithmetic, shifts, compound assignment, loops, calls, traps, and mixed-type rejection. The direct `u64` ELF example linked and returned 42 on `yicloud-dev`; early-return and bitwise regressions retained their results.

Numeric `for start..end` and `for start..=end` loops are supported for `i64` and `u64`; `usize` is an alias of `u64`. Endpoints are evaluated once, and the source loop variable is read-only in the body. Loop conditions are created after header Phis, hidden names are unique per nesting depth, and `continue` performs the induction increment before jumping. Inclusive ranges have a final-iteration guard so even `u64::MAX` exits without wrapping. Eleven integration tests cover empty and nested ranges, `break`, `continue`, early returns, endpoint calls, aliases, invalid syntax, and inclusive boundary cases. Real exclusive and inclusive `usize` examples returned 42 on `yicloud-dev`; unsigned-object and `break` regressions retained their results.

Scalar `as` casts are supported between `i64`, `u64`/`usize`, and `bool`. Signed/unsigned casts preserve bit patterns, boolean-to-integer casts produce 0/1, and numeric-to-`bool` casts are rejected. Casts bind before multiplication and may chain; they only update frontend type information and emit no instructions. Five integration tests cover round trips, booleans, precedence, chaining, calls, ranges, and invalid casts. The direct cast ELF example returned 42 on `yicloud-dev`; inclusive-range and unsigned-object regressions retained their results.

Unconditional `loop` supports the same `break` and `continue` paths as `while`. Function-tail literals, variables, arithmetic, booleans, casts, and calls no longer require an explicit `return`; the candidate check distinguishes the independent `return` token and statement keywords, and return types are still checked. Five integration tests cover unconditional loops, all supported tail classes, calls after range loops, and type errors. The real tail-call example returned 42 on `yicloud-dev`; cast and inclusive-range regressions retained their results.

Direct ELF emission now resolves local control flow. The encoder records label offsets and `rel32` fixups for `E9` and `0F 8x` jumps, computes displacement from the next instruction, and rejects duplicate/unknown labels and overflow. Three byte-exact tests cover backward and conditional jumps plus label errors; existing `if` and loop object tests now require success while preserving call relocation checks. On `yicloud-dev`, directly emitted `if`, loop, and early-return objects linked and returned 42, 55, and 42 respectively, without running an assembler.

Control-flow graphs no longer skip optimization wholesale. Pure unary, binary, and comparison nodes are folded in place, preserving node order and avoiding post-return constants; global value numbering merges equivalent pure expressions across regions without changing control edges. Trapping division remains intact. Four integration tests cover branch constants, cross-region numbering, loop conditions, and division traps. Optimized direct `if` and loop objects returned 42 and 55 on `yicloud-dev`, and the inclusive executable returned 42.

The control-flow optimizer now applies in-place algebraic identities and removes unused pure nodes. Node replacements count as progress only when they change at least one use, preventing non-convergent repeated rewrites; DCE roots control nodes, calls, and trapping divisions. Four integration tests cover arithmetic identities, zero and self operations, self-comparison, unsigned division by one, and preserved self-division traps. Optimized direct `if` and loop objects returned 42 and 55 on `yicloud-dev`, and the inclusive executable returned 42.

Immediate loads now use 7-byte sign-extended `C7 /0 imm32` for values fitting `i32`, rather than always emitting 10-byte `movabs`. Full 64-bit constants retain `B8+rd imm64`. Byte-exact tests cover 42, -2, and `i64::MAX`, and assembly/object tests were updated accordingly. Real direct `main`, `if`, and loop objects returned 42, 42, and 55 on `yicloud-dev`.

The emitter now suppresses self-assigned register moves after allocation. Three integration tests verify `if`, loop Phi, and call results without self moves. Real stack-argument validation exposed a separate direct-encoding bug: `[rsp+disp32]` lacked its required SIB byte. The encoder now emits `0x24`, with exact tests for `[rsp]` and `[rsp+8]`; the direct `sum8` object returned 36 on `yicloud-dev`, and `if`/loop objects retained 42/55.

Conditional branches now use a `Test` instruction that emits only `cmp`, avoiding unused `setcc` and `movzx` work and register definitions. Three tests distinguish branch conditions from actual boolean results. Multi-function validation also exposed cross-function value-numbering references; expression tables now reset at each function `Start`, with a regression test for the exact boolean helper/main shape. Real `if`, loop, and multi-function boolean programs returned 42, 55, and 42 on `yicloud-dev`.

Comparison-to-branch fusion now handles relational comparisons as well as equality. If all comparison uses are `If` conditions, the boolean node becomes a no-op and the branch compares the original operands with the inverse condition. Shared comparisons still materialize their values. Four tests cover `if`, loops, actual booleans, and shared comparisons. Real `if`, loop, and multi-function boolean programs returned 42, 55, and 42 on `yicloud-dev`.

Strength reduction now turns multiplication by a power of two into `shl` on either operand side and unsigned division by a power of two into `shr`. Signed division and non-power multiplication retain their original instructions. Four machine-level tests verify emitted instructions and results, including negative signed division. The direct `scale`/`reduce` example returned 42 on `yicloud-dev`, with `if` and loop regressions unchanged.

Shift lowering now preserves `Const` right operands as immediates and emits `C1 /4|5|7 imm8` instead of loading the count into `rcx`. Counts still mask to six bits, including negative counts. Three tests cover encoding, no-`cl` lowering, and count masking, plus exact bytes for all three shift directions. Real strength and control-flow objects retained their results on `yicloud-dev`.

Right-hand constants can now use immediate arithmetic, bitwise, comparison, and multiplication encodings. Their original constant nodes are removed only when every use is an eligible right operand; large values still materialize in registers. Five tests cover no-move arithmetic, bitwise/multiply/compare immediates, large fallback, exact `81`/`69` bytes, and rejected immediate division. Earlier spill fixtures now use real parameters so register-pressure coverage remains meaningful. Real strength, `if`, and `sum8` objects returned 42, 42, and 36 on `yicloud-dev`.

Commutative addition, multiplication, and bitwise operations now normalize a left constant into the immediate right-hand position, avoiding its register materialization. The swap is allowed only when the other operand is non-constant, so two-constant expressions cannot lose required values. Four tests cover left addition, multiplication, bitwise AND, and unchanged subtraction. Real strength, `if`, and `sum8` objects retained 42, 42, and 36 on `yicloud-dev`.

Phi edges now discard self-moves and emit acyclic copies directly, without allocating temporary virtual registers. Copies whose destinations occur in other source values retain temporaries to preserve parallel-copy cycles. Two tests check a loop with no temporaries and a synthetic `a ↔ b` Phi cycle that keeps them. Real direct `if`, loop, and `sum8` objects returned 42, 55, and 36 on `yicloud-dev`.

The current suite passes 504 tests on macOS and all 511 tests on `yicloud-dev` with Rust 1.99.1; Clippy passes on both hosts, and the release build passes on `yicloud-dev`. An earlier parallel remote run hit a transient `Text file busy` error in an unrelated historical wrapper audit test; later complete serial runs passed. Direct objects support both straight-line and control-flow code; executable output may still use system `as`/`gcc` when that path is selected. These results demonstrate a working native subset, not complete Rust language or whole-workspace support.
