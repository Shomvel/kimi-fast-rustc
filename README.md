# kimi-fast-rustc

`kfrc` is an experimental Rust compiler backend. The pinned rustc 1.94.1 frontend
provides typed MIR; the backend retains Rust aggregate, variant, pointer and
memory semantics, then explicitly lowers to x86-64 SysV and ELF objects.
Semantic IR is the sole production pipeline. There is no function-level LLVM fallback.

Eligible generic definitions now enter an immutable R graph once and specialize
on demand. Remaining constructs enter semantic S through concrete typed input;
coverage is reported explicitly. Answer-key deduplication and incremental caches
are still in progress.
Supported scope and measured results are recorded in
[scope](docs/plan/poc/scope.md) and [semantic status](docs/plan/poc/semantic-ir-status.md).
Passing correctness tests does not establish LLVM performance parity.

## Build and run

The backend requires Linux x86-64 and rustc 1.94.1 with `rustc-dev`:

```sh
cargo build --release
cd crates/rustc_codegen_kfrc
RUSTC_BOOTSTRAP=1 cargo build
cd ../..
export KFRC_CODEGEN_BACKEND="$PWD/crates/rustc_codegen_kfrc/target/debug/librustc_codegen_kfrc.so"
./target/release/kfrc native examples/native_main.rs -o native-main
./native-main
```

`kfrc native` invokes `rustup run 1.94.1 rustc` with the semantic backend,
`panic=abort` and MIR optimization disabled. `KFRC_RUSTC` can select an explicit
matching rustc binary. Other arguments use rustc syntax, for example
`--crate-type rlib --crate-name example -Copt-level=3`.
`KFRC_OPTIMIZE=0` disables graph optimization; `KFRC_INLINE=0` disables inlining.

The old handwritten subset frontend and `.kfr` execution entry have been removed;
examples now have ordinary Rust `main` functions. `native-batch` is no longer an
entry to a second compiler IR. Its historical benchmark results remain in evidence.

## Implementation

```text
rustc definition MIR → semantic R graph → demand-driven specialization → S
rustc concrete MIR → Typed Body → semantic S graph
  → aggregate/variant/scalar rules, memory promotion, semantic inlining
  → explicit layout L graph → machine instructions → register allocation → ELF
```

`crates/kfrc-body` owns frontend-neutral typed input and type facts;
`crates/kfrc-son` owns the semantic graph, verifier, interpreter and transforms;
`crates/kfrc-x86` owns machine lowering, encoding and object output.
The rustc shell links rustc-private APIs. The separate LLVM profiling wrapper
also links them to measure the unchanged reference backend; it is not a kfrc
code-generation fallback.
R/S/L are stages of the same IR design, not alternative compiler pipelines.

The implementation work follows the concrete programs and findings in
[W01–W10](docs/research/walkthroughs/README.md). The goal is to avoid creating
layout, copy, tag-decoding and specialization work that preserved semantics can
eliminate earlier. Indexing cost, rule visits, graph growth, memory operations,
compile instructions, runtime and text size are measured separately.

## Validation

```sh
cargo test --workspace
# In the pinned Linux validation environment:
bash crates/rustc_codegen_kfrc/tests/semantic-ir/run.sh "$KFRC_CODEGEN_BACKEND" /tmp/kfrc-semantic
bash crates/rustc_codegen_kfrc/tests/p3-semver/run.sh "$KFRC_CODEGEN_BACKEND" /tmp/kfrc-semver
```

Real Rust fixtures compare LLVM, semantic optimization off and optimization on.
ABI drivers, original crate tests and generated programs preserve observable
outputs and object provenance. Formal measurements use the same source, std,
toolchain and sandbox, following the frozen [metrics](docs/plan/poc/metrics.md).
Current evidence is in `evidence/poc-semantic-only-20261109/`.

The root wrapper's explicit LLVM benchmark, tuning and cache administration tools
remain for baseline experiments. Their results are not semantic-backend results.
