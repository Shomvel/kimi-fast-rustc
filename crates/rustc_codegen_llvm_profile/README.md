# LLVM instruction window

This separate rustc 1.94.1 backend delegates **every** `CodegenBackend` method to
the real `rustc_codegen_llvm::LlvmCodegenBackend`. It leaves LLVM optimization,
target options, codegen units, output formats, metadata and linking unchanged.
It is measurement infrastructure, never a fallback for kfrc compilation.

Build on Linux x86-64 with the matching `rustc-dev` toolchain:

```sh
RUSTC_BOOTSTRAP=1 cargo +1.94.1 build --release --manifest-path crates/rustc_codegen_llvm_profile/Cargo.toml --lib
```

Set `KFRC_LLVM_PROFILE=/absolute/output.jsonl` and load
`target/release/librustc_codegen_llvm_profile.so` from this package with rustc's
`-Zcodegen-backend=...`. The measured interval starts immediately before delegated
`codegen_crate` and ends after delegated `join_codegen`, including joined LLVM
worker work. It excludes the subsequent link operation. It includes host compiler
work between those entry points, so it must not be labeled pure LLVM pass time.

Before accepting results, compile and run the independent inheritance probe:

```sh
rustc +1.94.1 --edition=2024 -O crates/rustc_codegen_llvm_profile/src/bin/inherited_counter_probe.rs -o /tmp/inherited-counter-probe
/tmp/inherited-counter-probe
```

The probe covers the current thread, a newly spawned thread and grandchild, and a
worker that already existed before arming. Equal work must produce comparable
instruction counts. A nonzero exit means the counter is unavailable or coverage
failed; no comparison may use that run as performance evidence.

Each initial `/proc/self/task` thread gets an inherited, pinned `instructions:u`
event. Descendant tasks contribute to their initial root, including workers that
terminate before the read. A changing thread set during setup is rejected to
avoid missing tasks or counting them twice. Inheritance also includes any forked
child process started inside this window, if the backend starts one.

JSON records contain each root's count, enabled time and running time. Unavailable
PMUs, setup races, incomplete reads, multiplexing, an inactive current-thread
counter or zero total instructions produce `status: unavailable` without an
instruction field. Completely idle auxiliary roots may report three zeros.
Reporting failures go to stderr; missing or aborted records are never zero-cost
successes. Require one successful completed record for every intended crate.

Run profiled and unprofiled LLVM with identical arguments and compare their
observable output before using measurements. Keep performance runs separate from
correctness testing and record the toolchain, source and wrapper binary hashes.
