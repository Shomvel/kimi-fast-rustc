# Architecture

`kfrc` is a native-only compiler and never links to or invokes LLVM. It does not use Cranelift. Legacy LLVM modules and benchmark records are archival evidence only, not an active compiler path or fallback.

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
