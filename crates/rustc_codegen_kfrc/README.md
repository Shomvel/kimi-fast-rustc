# rustc_codegen_kfrc

PoC shell (`docs/plan/poc/`): rustc 1.94.1 runs its front end, this codegen
backend converts each mono item's MIR to a kfrc Typed Body, builds and
verifies a monomorphic SoN graph (`kfrc-son`), lowers it to x86-64 through
the selected machine layer, and hands one object per codegen unit
(`kfrc-object`) back to rustc's linker.

Machine layers (`docs/design/machine/layer.md`): `KFRC_MACHINE=cranelift`
(the default) exports the scheduled Layout graph to Cranelift (`kfrc-cranelift`);
`KFRC_MACHINE=x86` uses the original
selector (`kfrc-x86`) and serves as the differential baseline. Cranelift
switches: `KFRC_CRANELIFT_OPT=none|speed|speed_and_size` (default `speed`),
`KFRC_CRANELIFT_VERIFY=1` (run Cranelift's IR verifier and post-cleanup register-allocation checker),
`KFRC_CRANELIFT_PROMOTE=0` (keep all frame storage in memory),
`KFRC_CRANELIFT_SPLIT_CALLS=0` (one Cranelift block per scheduled block),
`KFRC_CRANELIFT_STORAGE_REPORT=1` (one `kfrc-storage` line per function).
`KFRC_DUMP_MACHINE=<symbol substring>` prints the Cranelift IR and machine
code of matching functions.

Inlining prices the simplified call-site body against the ABI call sequence.
The default is `KFRC_INLINE_COST_MODEL=callsite`: positive residual growth is
160 relative units at ordinary/cold/unknown sites and 256 in cyclic S control
components. Nested calls inherit a hot site only through classified non-cold
source paths. Constant-argument scratch preparation stays at 32; all existing
graph/depth/call/work limits still apply. These are bounded static hints, not
profile frequencies. Explicit `KFRC_INLINE_COST_MODEL=residual` retains uniform
32 pricing; any explicit `KFRC_INLINE_RESIDUAL_UNITS` selects that exact uniform
threshold, including when the model is otherwise `callsite`. Free ordinary calls retain
graph allocation and recursion checks while bypassing costed-call/depth limits.
`legacy`, `observe` and `size` remain calibration alternatives. See
[residual cost](../../docs/design/ir/inline-residual-cost.md) for the model and
its limits. `KFRC_CGSCC_STATS=1` enables cost, preparation and seam diagnostics;
`observe` always emits them.

The shell defaults to one costed inline level and at most 65,536 added nodes
per invocation. The library's standalone `InlineBudget` defaults are unchanged.
Calibration switches include
`KFRC_INLINE_DEPTH`, `KFRC_INLINE_CALLS`,
`KFRC_INLINE_COPY_CONSTANTS`, `KFRC_INLINE_LOOP_CONSTANT_BYTES`,
`KFRC_INLINE_INVOCATION_NODES`, `KFRC_INLINE_PREPARATION_WORK`.
The legacy `KFRC_INLINE_FREE_NODES` and `KFRC_INLINE_CALLEE_NODES` profitability
switches do not govern the residual policy. The shell uses
`KFRC_INLINE_INVOCATION_NODES` for its node allocation limit.

Layout lowering reuses predicates proved on incoming edges, peels one bounded
first iteration of eligible checked-multiply loops, and preserves captured
scalar niche representations when their exact layout permits copying them.
Set `KFRC_LAYOUT_EDGE_THREAD=0`, `KFRC_LOOP_PEEL=0`, or
`KFRC_SCALAR_NICHE_TRANSPORT=0` to disable each transformation for differential
diagnosis. See the [runtime validation](../../docs/validation/runtime-1p1-20261117.md)
for the original 13-workload comparison, ablations, and remaining LLVM gap.

Rust semantic values are the sole production IR. Every function goes through
`build_graph → verified semantic optimization → explicit layout lowering → machine layer`.
There is no legacy builder or IR mode switch. Unsupported input fails the crate.
The current path includes products, wide pointers, typed memory, dynamic enum
and niche ABI lowering, and bounded same-crate semantic inlining.
`KFRC_OPTIMIZE=0` skips optimization; `KFRC_INLINE=0` skips inlining only.
Owned Body/graph preparation and reachable function compilation run in bounded
workers. rustc queries and diagnostics remain on the compiler thread. Workers
borrow Cargo jobserver tokens, so `cargo -j1` can complete on the caller alone.
The default worker ceiling is the available CPU count;
`KFRC_CODEGEN_THREADS=N` lowers it (`1` selects serial execution, `0` is invalid).
This also permits independent functions in one CGU to run concurrently.
Function and immutable data visibility spans the crate. Each surviving function
is compiled once; its original CGU placement determines the object that owns it.
Cross-CGU internal references use hidden global symbols. See
[program data](../../docs/design/ir/program-data.md) for ownership and constant
identity.
All workers join before object emission or an error is reported. Diagnostic
graph dumps may interleave between functions; use one thread for ordered dumps.
See [the semantic status](../../docs/plan/poc/semantic-ir-status.md) and the
LLVM/semantic-off/semantic-on runner `tests/semantic-ir/run.sh` for verified coverage.

Eligible definitions use a cached polymorphic R graph and answer-driven
specialization; other instances enter through a concrete Typed Body. The scope is recorded in
[semantic-ir-migration.md](../../docs/plan/poc/semantic-ir-migration.md).

It links `rustc_private`, so it is outside the root workspace and pinned to
rustc 1.94.1 with `rustc-dev` (see `rust-toolchain.toml`). Linux x86-64 only.

```sh
cd crates/rustc_codegen_kfrc
RUSTC_BOOTSTRAP=1 cargo build
# A library crate compiled by kfrc:
RUSTC_BOOTSTRAP=1 rustc +1.94.1 --crate-type rlib -Cpanic=abort -Coverflow-checks=off \
    -Zmir-opt-level=0 -Zcodegen-backend=$PWD/target/debug/librustc_codegen_kfrc.so lib.rs
```

Tests (run on the HK host):

```sh
sh tests/p0-scalar/run.sh $PWD/target/debug/librustc_codegen_kfrc.so <work dir>
sh tests/p0-scalar/generated/run.sh $PWD/target/debug/librustc_codegen_kfrc.so <work dir> 1 30 40
sh tests/p0-entry/run.sh $PWD/target/debug/librustc_codegen_kfrc.so <work dir>
bash tests/p1-memory/run.sh $PWD/target/debug/librustc_codegen_kfrc.so <work dir>
```

Executables use the target C entry point, forwarding through Rust's start
lang item into the prebuilt `std::rt::lang_start_internal`. The PoC shell
currently supplies an adapter for `fn main() -> ()`: its capture is the main
function pointer, and its trait-object vtable is read from rustc's evaluated
allocation. Equivalent compiler-generated closure shims call that pointer and
return the unit `Termination` status, zero. Runtime initialization, argument
setup, panic handling and cleanup remain in the real std runtime. Other
`Termination` return types produce an explicit unsupported-input error.

Allocator wrappers come from rustc's `allocator_shim_contents` and internal
symbol mangling. They are emitted in a separate allocator module, forwarding
to std's default allocator and error handler when rustc requests those shims.
The entry probes check a `#![no_main]` executable (exit 37), an ordinary Rust
unit main (exit zero), and a Vec workload that allocates, grows beyond its
initial capacity, checks all values, and drops its allocation. Each compares
exit status, output, and object provenance with LLVM. A separate failure case
calls `handle_alloc_error` for a fixed layout and compares its abort and message;
it exercises the error-handler shim without exhausting the sandbox's memory.

The memory probes cover references, aggregates, slices, enum layouts, dynamic
dispatch and drop, and checked-arithmetic panic locations. Unsizing follows
rustc's field layouts through Box and custom CoerceUnsized wrappers, preserving
unchanged fields and zero-sized fields; struct-tail slices retain their prefix.
Callback probes exercise Fn/FnMut/FnOnce with function items, function pointers,
captures and track-caller adapters. A function item's type determines its direct
target even when MIR stores the zero-sized value in a local or aggregate field.

Debugging switches: `KFRC_DUMP_BODY=1`, `KFRC_DUMP_GRAPH=1`,
`KFRC_DUMP_MACHINE=<symbol substring>`, and
`KFRC_INTERPRET=<symbol substring>=<arg>,<arg>` (runs the reference
interpreter on the verified graph, to tell graph bugs from lowering bugs).
`tests/p0-scalar/generated/reduce.py` delta-reduces a mismatching
generated function.

The typed graph peephole optimizer runs after construction and before graph
verification. Set `KFRC_OPTIMIZE=0` to disable it for differential debugging;
`KFRC_DUMP_GRAPH=1` includes its statistics and the resulting graph.

Unsupported input is a fatal error naming the function and construct; see
`docs/plan/poc/p0-status.md` for the supported subset.

The converter keeps instance construction and MIR-local remapping in
`src/convert.rs`. Its child modules separate recorded target types/layouts,
places/constants, rvalues, statements, control flow, call ABI mapping and
intrinsic expansion. These modules translate MIR semantics; graph optimizations
belong to `kfrc-son` rather than this rustc adapter.
