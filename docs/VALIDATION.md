# Validation evidence

## Environment

Formal validation uses `yicloud-dev`, Linux x86_64, four available CPUs, and the official Rust 1.99.1 toolchain. Benchmark inputs are ripgrep 15.2.0 (`e89fff89ac9af12e8d4ce9d5fd07beb408ca730f`) and cargo 0.99.0 (`797e8a9bca276c1c9f9f738d2a20f484fa4eea9d`). Dependencies are fetched before timed builds and used offline.

## Confirmed native correctness

On `yicloud-dev`, the project test suite and Clippy passed: 235 tests, zero failures. The native executable path compiled `examples/native_main.kfr` through sea-of-nodes optimization, linear-scan allocation, GNU assembly, and system `as`/`gcc`; the resulting x86_64 executable returned 42.

The direct object path generated an ELF64 little-endian x86_64 relocatable object without an assembler. `readelf` confirmed a global `main` function symbol. A restricted-PATH run had no `as` available and still produced the object. Linking that object with GCC produced an executable that returned 42.

SoN/reference differential tests cover arithmetic, comparisons, wrapping boundaries, register and stack parameters, high register pressure, and spills. These tests do not replace the real x86_64 execution above.

## Whole-project build prechecks

Both pinned projects built successfully offline with stock Rust 1.99.1:

- ripgrep 15.2.0 produced `rg` and reported its expected version.
- cargo 0.99.0 produced `cargo` and reported its expected version.

These are build prechecks, not performance results.

## Formal benchmark status

Early attempts stopped before compilation because the fetch target and `/usr/bin/time` were unavailable. Those issues were fixed. A later run built all three ripgrep arms and Cargo's baseline/controlled arms, then stopped when ordinary LTO was rejected for a proc-macro crate. The proc-macro-specific filter is being fixed without disabling LTO for final binaries.

The successful ripgrep artifacts from `/root/kimi-fast-rustc-benchmarks/results/20261107T154033Z` were measured separately with 15 alternating samples:

| Arm | Binary bytes | Median runtime | Size ratio | Runtime ratio |
|---|---:|---:|---:|---:|
| baseline | 30,571,200 | 4.417 ms | 1.000 | 1.000 |
| controlled | 30,571,200 | 4.682 ms | 1.000 | 1.060 |
| candidate | 3,845,896 | 4.350 ms | 0.126 | 0.985 |

Baseline and controlled binaries have the same SHA-256, confirming exact controlled code generation; their runtime difference is sampling noise from short processes. The candidate is about 87.4% smaller and its median runtime is about 1.5% faster in this partial measurement. Cold compile wall times were 14.55 s baseline, 14.80 s controlled, and 28.57 s candidate, so the current ThinLTO candidate trades compile time for artifact size.

A larger 30-sample search over the Cargo source tree gave median runtimes of 11.157 ms baseline, 11.158 ms controlled, and 11.203 ms candidate. The controlled ratio was 1.00007 and the candidate ratio 1.00415. This larger workload is approximately at parity but does not meet the strict `candidate / baseline <= 1.0` rule, so it is not presented as a confirmed runtime win.

These are valid partial LLVM reference-track results, not a completed two-project formal run. Full three-arm Cargo results and a final combined run remain pending. Native whole-project support is not implied by these results.
