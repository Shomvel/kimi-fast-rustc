# Architecture

`kfrc` has two independent compiler tracks: an LLVM reference track and a native track that does not link to or invoke LLVM. Neither track uses Cranelift.

## LLVM reference track

Cargo invokes `kfrc` as a rustc wrapper. The driver preserves rustc's wrapper protocol, forwards diagnostics, and selects LLVM optimization settings. Work is pipelined through Cargo's crate graph and a bounded local task scheduler. Reproducible benchmarks compare stock rustc/LLVM with kfrc on the same host and toolchain. This track supplies the compatibility and performance baseline; it is not part of the native backend implementation.

## Native backend path (no LLVM)

The native backend uses a sea-of-nodes graph: nodes represent computations and edges represent data or control dependencies. Nodes do not inherently have an execution order. Optimization rewrites the graph, then an explicit block-placement pass puts control flow into basic blocks. This pass does not perform ILP scheduling or reorder instructions within a block. Instruction selection preserves lowering order and relies on CPU out-of-order execution for dynamic scheduling. Linear-scan register allocation maps virtual registers to target registers and inserts spills. The initial native target is x86_64 System V on Linux, with no LLVM libraries, tools, or subprocesses in this path.

The native backend must never silently change Rust semantics. Unsupported MIR constructs are rejected explicitly while the backend is experimental; production builds use the LLVM path. Correctness tests compare native execution with the source-level expected result before performance claims are made.

## Constraints

- No Cranelift.
- No disabling optimization to make compilation appear faster.
- No unsafe floating-point reassociation by default.
- `target-cpu=native` results are reported separately from portable results.
- Binary size and runtime are both first-class metrics.
- All official builds and benchmarks run on SSH host `yicloud-dev`.
- Modules use only the Rust standard library unless the root manifest is deliberately changed by the integrator.
