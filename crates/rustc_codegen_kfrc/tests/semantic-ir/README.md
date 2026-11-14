# Semantic IR machine differential

Run on Linux x86-64 with the pinned rustc 1.94.1 toolchain:

```sh
bash crates/rustc_codegen_kfrc/tests/semantic-ir/run.sh /absolute/backend.so /fresh/result/directory
```

The same library source runs through LLVM, semantic kfrc without optimization, and semantic kfrc with optimization.
The semantic pipeline is the sole kfrc compilation path; there is no IR selector.
Every arm uses the same LLVM ABI provider and LLVM driver. The driver checks
expected values as well as producing byte-for-byte comparable output.

Coverage includes nested field extraction/update, immutable snapshots, aggregate
branch/loop Phis, array destructuring, local calls, and tuple/struct parameters
and returns. Known enum variants are constructed and matched without layout
encoding; wide slice references pass through selection and return, including
empty slices. Pair and large indirect values cross both directions: the LLVM
driver calls kfrc functions which call non-inlined LLVM provider functions.
No unsafe memory access or fabricated ABI declaration is used.

Each semantic graph must produce both S-input and L-output dump markers; ELF
objects must identify the kfrc writer. Borrow, dynamic-index, dynamic-enum-argument and niche-enum probes currently
expect explicit compilation failure for the whole crate, even alongside a
supported function. When those capabilities are implemented, move these probes
to positive differential coverage instead of silently removing their checks.

This slice does not establish full polymorphic R/S, semver acceptance of the new
chain, or performance. Raw commands, statuses, dumps, hashes and the generated
summary remain in the output directory.
