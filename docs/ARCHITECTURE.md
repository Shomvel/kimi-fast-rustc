# Architecture

`kfrc` is a native-only compiler and never links to or invokes LLVM. It does not use Cranelift. Legacy LLVM modules and benchmark records are archival evidence only, not an active compiler path or fallback.

## Native backend path (no LLVM)

The native backend uses a sea-of-nodes graph: nodes represent computations and edges represent data or control dependencies. Nodes do not inherently have an execution order. Optimization rewrites the graph, then an explicit block-placement pass puts control flow into basic blocks. This pass does not perform ILP scheduling or reorder instructions within a block. Instruction selection preserves lowering order and relies on CPU out-of-order execution for dynamic scheduling. Linear-scan register allocation maps virtual registers to target registers and inserts spills. The initial native target is x86_64 System V on Linux, with no LLVM libraries, tools, or subprocesses in this path.

The native backend must never silently change Rust semantics. Unsupported Rust constructs are rejected explicitly while the backend is experimental; there is no fallback compiler path. The frontend tracks `i64`, `u64`, and `bool` types in bindings, expressions, and function signatures. Unsigned literals can be inferred from unsuffixed literals in a `u64` context, but concrete signed and unsigned variables cannot be mixed. Conditions require `bool`; logical `&&` and `||` place the right expression on the required control path and merge it with a constant on the other path. Correctness tests compare native execution with the source-level expected result before performance claims are made. Mutable locals are versioned as SSA values. Every reachable `if` arm ends in an explicit `Jump` to its merge region, so nested statements do not depend on physical branch layout. A `while` header accepts its entry and any number of back edges from ordinary completion and `continue`. Each `break` records an exit jump with the current SSA values; a loop with breaks merges those edges with the normal condition exit. Phi values are copied through temporaries before assignment on every edge, and live ranges that cross a loop header extend through its back edges. Functions may contain multiple `Return` nodes; instruction selection emits each return and its register restoration, while the last return delimits each function graph for multi-function compilation. Single-function lowering explicitly rejects multiple `Start` nodes. Integer bitwise operations use the ordinary two-address binary path. Shifts preserve the left input in scratch before loading the count into `cl`, then use `shl`, arithmetic `sar`, or logical `shr`. Unsigned division clears `rdx` and uses `div`, while signed division uses `cqo` and `idiv`; direct object emission encodes the same operations without an assembler.

## Constraints

- No Cranelift.
- No disabling optimization to make compilation appear faster.
- No unsafe floating-point reassociation by default.
- `target-cpu=native` results are reported separately from portable results.
- Binary size and runtime are both first-class metrics.
- All official builds and benchmarks run on SSH host `yicloud-dev`.
- Modules use only the Rust standard library unless the root manifest is deliberately changed by the integrator.
