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
