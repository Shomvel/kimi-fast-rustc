# kimi-fast-rustc

`kfrc` is an experimental Rust compiler with two strictly separated tracks:

- **LLVM reference track:** a Cargo-compatible `RUSTC_WRAPPER` using rustc and LLVM. It supports controlled pass-through, reproducible optimization profiles, PGO command modeling, artifact analysis, and benchmarks.
- **Native track:** a sea-of-nodes IR, graph rewrites, single-block lowering, linear-scan register allocation, and x86_64 System V assembly emission. This track does not use LLVM or Cranelift and does not perform static ILP instruction scheduling. It relies on CPU out-of-order execution after semantics-preserving lowering.

The native frontend currently accepts only a documented, single-basic-block `i64` Rust expression subset. Unsupported Rust is rejected; it never silently falls back to LLVM. Full Cargo projects such as ripgrep and cargo currently require the LLVM reference track. No native performance or whole-project parity claim is made.

## Build

```sh
cargo build --release
cargo test
cargo clippy --all-targets -- -D warnings
```

## Use the LLVM reference track

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

Set `KFRC_TARGET_CPU=native` for a host-specific build. Such results must be reported separately from portable builds. `KFRC_BACKEND=native` rejects full Cargo compilation rather than silently selecting LLVM.

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
```

The direct object path uses the in-tree x86_64 encoder and ELF writer. External references use explicit relocation records and remain limited to the implemented ABI subset. Set `KFRC_NATIVE_CACHE` to a dedicated directory to enable content-addressed object caching; graph, target, ABI, and backend-version changes create distinct entries. Caching is disabled by default.

Compile multiple independent inputs with bounded parallelism:

```sh
./target/release/kfrc native-batch --jobs 4 --out-dir objects examples/native_main.kfr examples/native_second.kfr
# Add --pipeline to overlap parsing, graph optimization, and code generation.
./target/release/kfrc native-batch --pipeline --jobs 4 --out-dir objects examples/native_main.kfr examples/native_second.kfr
# Inspect or safely clear a dedicated native object cache.
./target/release/kfrc cache --root .kfrc-cache stats --json
```

The CLI performs sea-of-nodes optimization, dependency placement, linear-scan allocation, and native code generation. Executables use system `as` and `gcc`, never rustc, LLVM, or Cranelift. The subset requires an explicit `return` and supports `i64` parameters, immutable `let` bindings, arithmetic, and comparisons. Its `fn main() -> i64` convention is intentionally a compiler test format, not a claim that standard Rust accepts this `main` signature.

## Benchmark on yicloud

The repository includes `scripts/bench-yicloud.sh`. It builds ripgrep and cargo from recorded commits using separate target directories and three arms: stock rustc, `controlled` wrapper pass-through, and the optimized candidate. It records compiler and host fingerprints, measures binary sizes and compile times, and alternates all three binaries during runtime sampling. The script expects:

- SSH host: `yicloud-dev`
- Rust 1.99.1 toolchain: `/root/kfrc-toolchain-1.99.1/bin`
- Benchmark sources: `/root/kimi-fast-rustc-benchmarks`
- ripgrep 15.2.0: commit `e89fff89ac9af12e8d4ce9d5fd07beb408ca730f`
- cargo 0.99.0: commit `797e8a9bca276c1c9f9f738d2a20f484fa4eea9d`
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
