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

The lexer now accepts decimal, binary, octal, and hexadecimal integer literals with underscores and `i64`/`isize`/`u64`/`usize` suffixes. `isize` maps to the signed 64-bit word type on the current x86_64 target. Unsuffixed literals retain their full 64-bit magnitude until typed, allowing an inferred `u64::MAX` while preserving signed-range errors. Compound assignment now includes `&=`, `|=`, `^=`, `<<=`, and `>>=`. Five integration tests cover radices, aliases, differential arithmetic, unsigned operations, loop Phis, malformed literals, and type errors. The direct `integer_syntax` object linked and returned 42 on `yicloud-dev`; `if`, loop, and `sum8` regressions retained 42, 55, and 36.

Function signatures are now collected before bodies are parsed. Declaration-order indices give arbitrary forward calls, self-recursion, and mutual recursion the correct `Call` targets while retaining arity and type checks. Four integration tests cover non-adjacent forward calls, both recursion forms, nested bodies, and invalid calls. On `yicloud-dev`, the recursive example produced five verified `R_X86_64_PLT32` relocations, linked into a real executable, and returned 42 after exercising self- and mutual recursion.

Semicolon-terminated expression statements now accept unit and non-unit calls as well as discarded pure expressions. Unit calls produce an internal `()` type that can be bound or returned from unit functions but is rejected in arithmetic, comparisons, casts, and typed results. Four integration tests cover argument passing, discarded values, unit bindings, object emission, and type errors. The real expression-statement example returned 42 on `yicloud-dev` after executing a unit loop function and ordinary calls.

Unary `!` now dispatches by type: booleans retain logical negation, while signed and unsigned integers use the existing SoN bitwise `Not`, machine `not`, constant-folding, and value-numbering paths. Four integration tests cover signed and unsigned boundaries, precedence, double inversion, loop Phis, native objects, unit rejection, and signed literal overflow. The real mixed signed/unsigned example returned 42 on `yicloud-dev`.

Unsigned remainder by a power of two now strength-reduces to a bitmask, and multiplication/division/remainder reduction covers the `2^63` divisor. Replacement constants are newly allocated rather than mutating shared constant nodes; a shared `/ 8` and `% 8` regression guards the original divisor semantics. Three integration tests cover masks, shared constants, and high-bit operations. The direct remainder object returned 42 on `yicloud-dev`. Variable shift lowering also removed the fixed `r11` to `rcx` copy, with one instruction-count regression and real execution coverage.

Function-level bounded parallel compilation preserves declaration order and emits byte-identical assembly and ELF objects for 1, 2, 3, and 8 jobs, including forward and recursive calls. The native CLI exposes `--jobs N` and `--jobs=N`, rejects zero jobs, and defaults to available parallelism. Move-guided linear-scan allocation accepts only first-definition/dead-source moves, with 12 tests covering move chains, spills, fixed registers, Phi back edges, and deterministic invalid hints.

A six-example yicloud measurement used three warmups and 30 serial samples per mode. Direct-object compile medians ranged from 0.525 to 0.739 ms; full executable compile medians including assembler/linker subprocesses ranged from 15.258 to 15.929 ms. Direct objects were 616–1512 bytes and linked executables 15,744–15,832 bytes, with the documented 42/55/36 execution results. These are small native-subset workloads, not whole-project performance results.

Boolean `==` and `!=` now share the existing comparison lowering, with eight tests covering truth tables, short-circuiting, branch/loop Phis, calls, and mixed-type errors. Real x86_64 execution exposed a non-dominating global-value-numbering reuse: a loop-body `!value` incorrectly replaced the loop-exit `value == false`. Value numbering now computes block dominators and only reuses non-constant expressions in the same or a dominating block. Five regressions cover the original graph, arithmetic/bitwise/comparison expressions, safe dominating reuse, branch-local values, and loop exits; the final real example returned 42.

Function subgraph construction also preserves and remaps optimization-appended dependencies outside the original contiguous function range. This fixes both constant-folded visibility functions and the recovered `later(40) + twice(1)` strength-reduction failure. Visibility tests cover four accepted modifiers and optimized multi-function constants; parallel tests cover the recovered source with byte-identical output.

Compact x86 encoding uses signed-imm8 group-1/`imul`/frame encodings and short RBP/RSP displacements, with correct SIB and REX bits. Six new vectors plus updated immediate/stack tests cover encoding boundaries, local jumps, and call relocations. Final direct objects include `sum8` at 984 bytes, recursion at 1288 bytes, visibility at 728 bytes, and boolean comparison at 784 bytes. Eight final real programs returned their expected 42/55/36 results on `yicloud-dev`; visibility and recursion objects were byte-identical between jobs 1 and 4.

`let` and loop-variable shadowing now preserve independent bindings through RHS evaluation, branch Phis, loop back edges, breaks, and outer-scope restoration. Six tests cover these cases and a real example. Local branches iteratively use `EB`/`7x rel8` when in range, with five boundary/layout tests including final call-relocation offsets. The layout engine now uses prefix deltas and one final linear compaction; six tests and 1,396 differential cases preserve exact code and relocations, while dense synthetic workloads improved from 71.0 to 0.81 ms and 87.9 to 1.58 ms respectively. Nested boolean comparisons simplify to original or inverse comparisons, with four signed/unsigned and branch tests; the real inverse example returned 42. Zero comparisons now use `85 /r` test encodings on both comparison paths, with five tests covering all ten conditions, exact extended-register bytes, assembly, spills, and direct objects.

Function workers dynamically claim indices instead of using static strides. Seven integration tests cover uneven workloads, deterministic errors, and byte identity. Ordered spill expiration passed 15,000 baseline-differential cases and reduced a 50,000-spill allocation from about 2 seconds to 16 milliseconds in the recorded synthetic test. Immediate-dominator chains use O(V) storage and reduced the 256-diamond regression from 2.03 seconds to 0.90 seconds. A separate 32-function yicloud workload gave median compile times of 6.343/5.343/5.037 ms at jobs 1/2/4, with 27 byte-identical objects all executing correctly.

`const fn` supports all existing visibility modifiers and ordinary forward/recursive calls without claiming compile-time evaluation. Six tests cover modifiers, symbol indices, invalid syntax, and native execution. Top-level typed scalar constants support literals, negative values, radices, suffixes, visibility, local shadowing, and compile-time expression evaluation. Eight item tests cover namespace conflicts, function indices, type errors, and unsupported calls. Nine expression tests cover dependency ordering, cycles, precedence, short-circuiting, overflow, invalid shifts, and native execution. Native executables now link direct objects with GCC without an assembler; a nine-sample recursion comparison measured 14.383 ms for the default executable and 1.294 ms for direct-object compilation. The historical as+gcc comparison was 15.466 ms, so the cross-run difference is about 7%, not a controlled causal measurement. The native cache backend version advanced to v2 to reject pre-short-jump objects.

The three-stage batch pipeline uses typed bounded synchronous channels and configurable workers per stage, with contiguous-prefix output ordering. Tests cover direct object byte identity, capacity errors, panic stages, out-of-order completion, and bounded prefetch. The earlier single-worker synthetic executor improved by 1.092–1.259×, but its 16,384-input real-source comparison was only 1.004×. The newer multi-worker path improved two 64-input real-source comparisons by 1.832× and 2.108× respectively.

In-place unary strength rules cover zero subtraction, multiplication by minus one, and XOR with all ones on either operand side. Three integration tests verify signed and unsigned boundary values, including `i64::MIN`; the real example returned 42 and produced a 488-byte object on `yicloud-dev`.

Signed division by positive powers of two now uses a sign-derived bias and arithmetic shifts instead of `idiv`, preserving rounding toward zero and every existing trap. Six tests cover positive/negative inputs, `i64::MIN`, divisors through `2^62`, the negative `2^63` bit pattern, spills, and direct objects. The real signed-division example returned 42 and produced a 488-byte object on `yicloud-dev`.

A 64-helper, 32,768-operation yicloud workload compared 16 complete inputs over nine interleaved samples. Default direct-linked executables took 1133.331 ms versus 951.192 ms for direct objects, a 19.1% difference that includes GCC linking; 176 objects were byte-stable and all 352 execution checks passed. This is a native subset workload, not a batch or whole-project speedup.

`native-batch` now emits one complete program object per input, including forward and recursive functions with `R_X86_64_PLT32` relocations. Single-function inputs retain filename-stem symbols, and input-internal compilation stays single-threaded to avoid multiplying worker budgets. Tests compare DAG/pipeline objects at jobs 1/2/4 and execute forward/factorial programs returning 42/120 on Linux x86_64.

Scalar `match` supports signed/unsigned and boolean literals, negative values, nested expressions, blocks, and a required final wildcard. Twelve tests cover type checking, duplicate patterns, multiway Phis, aliases, and shadowing. Zero idioms use XOR only when flags are dead, with eight tests covering exact bytes, every condition, calls, returns, labels, and loops. Left-constant comparisons now swap into right-immediate form with the correct inverse operator. Four tests cover signed/unsigned operators, immediate elision, and two-constant folding; the real cross-function example returned 42 and produced a 728-byte object. Unsigned `0x80000000..=0xffffffff` constants now use zero-extending `B8 imm32`, saving another 4–5 bytes per load. Three tests cover boundary values, RAX/R8/R15, exact bytes, and direct object execution.

Inclusive match ranges and boolean guards now lower with signed/unsigned endpoint checks and control-dependent guard execution. Eleven tests cover full type boundaries, overlap errors, nested shadowing, guard traps, and real execution.

Value numbering now batches replacements into one graph scan per nonempty pass. A 1024-level repeated-expression regression preserves exact nodes and native output while reducing release time from 13.855 ms to 1.146 ms (about 12.1×).

`native_microbench.py` now provides deterministic multi-function inputs, JSON host/timing records, cross-job object hashes, and real execution checks in native, batch-dag, and batch-pipeline modes. On yicloud, jobs 1/2/4 produced identical objects for all generated inputs, and every linked program returned 42. One three-sample four-input run gave jobs=4 medians of 3.565 ms native, 1.142 ms batch-dag, and 1.259 ms batch-pipeline.

Restricted LICM hoists only pure non-division expressions with a unique loop preheader and dominating, loop-independent operands. Five tests cover zero/multiple iterations, nesting, non-dominating branches, memory/Phi exclusions, and division traps. A 64-helper runtime comparison gave 1.179× speedup and 15.3% smaller objects with 256 hoists, while the no-invariant group was 1.1% slower. Multi-function caching validates complete graph/function/Call keys and object formats; jobs=1 showed a 1.21× cache-hit benefit in one 16-function measurement, but parallel hash overhead made jobs 2/4/8 slower, and a larger warm-cache comparison differed by only about 0.4%. Deterministic shard hashing later improved hit time by about 50% at jobs=4 and 60% at jobs=8 in a 97-function comparison, while jobs=1 was flat and its miss path regressed about 7%; end-to-end optimizer time still dominates large CLI builds. No general cache speedup is claimed.

Function subgraph construction now scans only the current range and required external dependencies. Two deterministic regressions preserve holes, external constants, all input channels, metadata, and ELF bytes; the 96-function timing comparison improved by 6.74× and 6.84×, and a separate 64-function yicloud extraction comparison improved by 5.66×.

The final execution matrix covered shadowing, const functions/items, inverse and ordinary boolean comparisons, remainder, bitwise NOT, sum8, if, while, visibility, and recursion. Every direct object linked and returned its expected result on `yicloud-dev`; the default const-fn executable also returned 42 without an assembler. Objects now include const-fn/item at 488 bytes, shadowing at 664 bytes, zero-comparison if/while at 600/744 bytes, and recursion at 1256 bytes. Visibility and recursion remained byte-identical between jobs 1 and 4.

The current suite passes 687 tests on macOS and all 697 tests on `yicloud-dev` with Rust 1.99.1; Clippy passes on both hosts, and the release build passes on `yicloud-dev`. An earlier parallel remote run hit a transient `Text file busy` error in an unrelated historical wrapper audit test; later complete serial runs passed. Direct objects support both straight-line and control-flow code; executables are linked with system `gcc` without an assembler. These results demonstrate a working native subset, not complete Rust language or whole-workspace support.
