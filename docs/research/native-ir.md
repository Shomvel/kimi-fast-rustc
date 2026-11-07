# Native IR research

The native track lowers a strict Rust subset to a sea-of-nodes graph. Control, data, and future memory-state dependencies are distinct edge channels. Pure nodes have no inherent execution order; an explicit dependency-preserving block-placement pass produces a legal order before lowering. This is not ILP scheduling.

The initial implementation supports constants, parameters, integer operations, comparisons, `Start`, `Return`, and `Phi` at IR level. The executable backend deliberately accepts only single-basic-block operations it can lower exactly. Division traps, overflow behavior, pointer provenance, aggregate layout, unwinding, and unsupported control flow must not be approximated.

A production Rust mapping should use rustc's MIR after type checking, borrow checking, and monomorphization. Region nodes and Phi inputs must remain aligned with CFG predecessors. Addressable locals require memory-state edges; non-SSA locals cannot be treated as unrestricted SSA values. Generic functions require monomorphization before native code generation. Stable MIR or a version-matched rustc backend interface can supply this input, but neither is yet a full native frontend.

Graph optimizations preserve trapping division/remainder and memory dependencies. New graph nodes require dependency placement before lowering. Differential execution against the SoN reference interpreter is the initial correctness oracle; project test suites are required before broader claims.

References:

- Click and Paleczny, [A Simple Graph-Based Intermediate Representation](https://grothoff.org/christian/teaching/2007/3353/papers/click95simple.pdf).
- Braun et al., [Simple and Efficient Construction of Static Single Assignment Form](https://c9x.me/compile/bib/braun13cc.pdf).
- Rust Compiler Development Guide, [MIR](https://rustc-dev-guide.rust-lang.org/mir/index.html) and [monomorphization](https://rustc-dev-guide.rust-lang.org/backend/monomorph.html).
- Rust standard library, [pointer provenance](https://doc.rust-lang.org/std/ptr/index.html#provenance).
