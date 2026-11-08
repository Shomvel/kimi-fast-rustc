# kimi-fast-rustc

`kfrc` is an experimental, native-only Rust compiler. It uses a sea-of-nodes IR, graph rewrites, explicit control regions, linear-scan register allocation, and x86_64 System V code generation. It does not use LLVM or Cranelift and does not perform static ILP instruction scheduling; dynamic scheduling is left to CPU out-of-order execution.

The frontend currently supports a documented `i64` (`isize`), `u64` (`usize`), and `bool` Rust subset: decimal, binary, octal, and hexadecimal integer literals; mutable and immutable bindings and parameters, including `let` and loop-variable shadowing; ordinary, arithmetic, bitwise, and shift compound assignments; `as` casts between supported scalar types; arithmetic including wrapping negation; comparisons, bitwise operators and integer `!`, shifts, and logical `!`/`&&`/`||` with short-circuit evaluation; `if` expressions and statements with `else if`, plus scalar `match` with literal and inclusive-range arms, optional guards, and a required final unguarded wildcard; `while`/`loop` and exclusive/inclusive numeric `for` loops with `break`/`continue`, including unconditional `loop` expressions with valued `break`; early `return` in conditionals and loops; tail expressions and semicolon-terminated expression statements, including unit calls and discarded values; standard `fn main`; `pub`, `pub(crate)`, `pub(self)`, and `pub(super)` function visibility, top-level scalar constants with compile-time expression evaluation, and `const fn` runtime calls; and declaration-order-independent function calls, including self- and mutual recursion, with register and stack arguments. Unsupported Rust is rejected explicitly rather than falling back to another backend. Whole-workspace support for large projects such as ripgrep and cargo is not yet implemented; native measurements are limited to the workloads documented in `docs/VALIDATION.md`.

Legacy LLVM modules and benchmark data are retained only as historical evidence. They are not the current compiler architecture or a supported fallback.

## Build

```sh
cargo build --release
cargo test
cargo clippy --all-targets -- -D warnings
```

## Historical LLVM reference track

These commands are retained only to reproduce the archived comparison above; they are not the native compiler path or a fallback.

Direct compilation:

```sh
./target/release/kfrc rustc examples/hello.rs -O -o hello
```

Cargo wrapper:

```sh
RUSTC_WRAPPER="$PWD/target/release/kfrc" \
KFRC_PROFILE=controlled \
cargo build --release
```

Profiles:

- `controlled`: preserve rustc arguments exactly; this isolates wrapper overhead.
- `balanced`: opt-level 3, ThinLTO, one codegen unit, generic target CPU, symbol stripping.
- `speed`: opt-level 3 and the optimized reference configuration.
- `size`: optimize for size with ThinLTO and symbol stripping.

Set `KFRC_TARGET_CPU=native` for a host-specific build. Such results must be reported separately from portable builds. `KFRC_CODEGEN_UNITS=N` overrides the code-generation unit count, and `KFRC_LTO=thin|fat` selects the LTO mode while proc-macro crates remain filtered for compatibility. `KFRC_BACKEND=native` rejects full Cargo compilation rather than silently selecting LLVM.

## Use the native backend

On Linux x86_64, compile the supported `.kfr` Rust subset to an executable:

```sh
./target/release/kfrc native examples/native_main.kfr -o native-main
./native-main
test "$?" -eq 42
```

Emit assembly without running an assembler:

```sh
./target/release/kfrc native examples/native_main.kfr --emit=asm=native-main.s
```

Emit an ELF relocatable object directly, without an external assembler:

```sh
./target/release/kfrc native examples/native_main.kfr --emit=obj=native-main.o
# Compile independent functions from one input in parallel.
./target/release/kfrc native examples/native_recursion.kfr --jobs 4 --emit=obj=native-recursion.o
```

The direct object path uses the in-tree x86_64 encoder and ELF writer. Local labels and conditional/unconditional jumps use `rel8` when the final displacement fits and `rel32` otherwise, with fixups resolved after iterative layout shortening; external calls use explicit `R_X86_64_PLT32` relocation records and remain limited to the implemented ABI subset. Set `KFRC_NATIVE_CACHE` to a dedicated directory to enable content-addressed object caching; graph, target, ABI, and backend-version changes create distinct entries. Caching is disabled by default.

Compile multiple independent inputs with bounded parallelism:

```sh
./target/release/kfrc native-batch --jobs 4 --out-dir objects examples/native_main.kfr examples/native_second.kfr
# Add --pipeline to overlap parsing, graph optimization, and code generation.
./target/release/kfrc native-batch --pipeline --jobs 4 --out-dir objects examples/native_main.kfr examples/native_second.kfr
# Inspect or safely clear a dedicated native object cache.
./target/release/kfrc cache --root .kfrc-cache stats --json
```

The CLI performs sea-of-nodes optimization, dependency placement, linear-scan allocation, and native code generation. Executables are linked from the in-tree ELF object with system `gcc`, without an assembler; GNU assembly output still uses `as` and `gcc` when assembling or linking that representation. Neither path invokes rustc, LLVM, or Cranelift. The subset supports explicit returns and tail expressions, mutable and immutable `let` bindings, assignments, `while` loops, arithmetic, and comparisons. Its `fn main() -> i64` convention is intentionally a compiler test format, not a claim that standard Rust accepts this `main` signature. Direct ELF output supports local labels and branches as well as external call relocations.

## Native microbenchmark

`native_microbench.py` generates deterministic four-function inputs, checks object hashes across jobs, links and executes each result, and reports JSON timing and host fingerprints:

```sh
python3 scripts/native_microbench.py --compiler /root/kimi-fast-rustc/target/release/kfrc --jobs 1,2,4 --samples 7 --inputs 8
# Select batch mode and cold/warm cache behavior when needed.
python3 scripts/native_microbench.py --compiler /root/kimi-fast-rustc/target/release/kfrc --mode batch-pipeline --cache warm --jobs 1,2,4 --samples 7
```

Use `--skip-run` only on hosts that cannot execute Linux x86_64 output. The workload is a small native-subset benchmark, not a whole-project performance claim.

## Benchmark on yicloud

The repository includes `scripts/bench-yicloud.sh`. It builds ripgrep and cargo from recorded commits using separate target directories and three arms: stock rustc, `controlled` wrapper pass-through, and the optimized candidate. It records compiler and host fingerprints, measures binary sizes and compile times, and alternates all three binaries during runtime sampling. The script expects:

- SSH host: `yicloud-dev`
- Rust 1.99.1 toolchain: `/root/kfrc-toolchain-1.99.1/bin`
- Benchmark sources: `/root/kimi-fast-rustc-benchmarks`
- ripgrep 15.2.0: commit `e89fff89ac9af12e8d4ce9d5fd07beb408ca730f`
- cargo crate 0.99.0 (CLI 1.98.0): commit `797e8a9bca276c1c9f9f738d2a20f484fa4eea9d`
- `kfrc`: `/root/kimi-fast-rustc/target/release/kfrc`

Example on the remote host:

```sh
export PATH=/root/kfrc-toolchain-1.99.1/bin:$PATH
export CARGO_HOME=/root/kfrc-cargo-home
export CARGO_NET_OFFLINE=true
/root/kimi-fast-rustc/scripts/bench-yicloud.sh
```

Formal comparisons must use the same rustc, sysroot, source commit, lockfile, target, features, dependency cache, and CPU budget. Runtime and size ratios are candidate divided by baseline; a ratio at or below 1.0 is non-regression. Native results must never be presented as LLVM results or vice versa.

The completed `20261107T155940Z` run met both criteria for the LLVM candidate: ripgrep's size/runtime ratios were 0.126/0.931, and cargo's were 0.583/0.988. See `docs/VALIDATION.md` for all three arms, compile-time costs, native execution evidence, and the distinction between full-project LLVM results and the native subset.

## Native design

```text
Rust subset
  -> sea-of-nodes graph
  -> graph optimization
  -> explicit single-block placement
  -> order-preserving instruction selection
  -> liveness and linear-scan allocation
  -> spill insertion
  -> x86_64 System V GNU assembly
```

There is no machine-instruction list scheduler. The block scheduler under `scheduler.rs` is a bounded parallel task scheduler for compiler work, not an ILP scheduler.

See `docs/ARCHITECTURE.md` for the track boundary and correctness rules.
