# LLVM parity library suite

Pinned library versions, formal flags, LLVM-built drivers, corpus/checksum comparisons,
upstream tests and 11-round runtime measurements are defined in `suite.json` and `suite.py`.
`prepare` verifies locally supplied public crate archives; it does not download them.
Before running it, place each archive at `crates/NAME-VERSION/NAME-VERSION.crate`
relative to this suite directory, using the versions and SHA-256 values in `suite.json`. No dependencies or result binaries are vendored.

Use Rust 1.94.1 with rustc-dev/rust-src, the project's shared MIR-0 sysroot, an x86_64 Linux host,
and a working perf binary (the configured path is environment-specific).
Run `python3 suite.py --help` for prepare/arm/runtime/calibrate commands.
The shell wrappers resolve the suite sources relative to their own directory.

The accepted run first performed checker=1 correctness preflight, then measured checker=0
whole-process compilation in three forward/reverse/forward rounds. `cost-check0.py` records
that exact separate phase; its `/workspace` roots and backend paths are the retained run's
paths and must be updated for a new environment. `suite-phased.py` records the preflight/runtime phase.
Keep compilation, runtime and exact profiling separate and serial within each sandbox.

Generate a result table with `python3 gate_table.py RESULT_ROOT --arms llvm,c12-phi,integrated`.
Missing driver/corpus/workload/upstream evidence is incomplete; output differences are failures.
Run the eight reporting regression cases with `python3 test_gate_table.py`.

The accepted measurements and limitations are in
[`docs/plan/poc/llvm-parity/results/20261110`](../../docs/plan/poc/llvm-parity/results/20261110/README.md).
