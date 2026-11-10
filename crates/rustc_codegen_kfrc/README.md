# rustc_codegen_kfrc

PoC shell (`docs/plan/poc/`): rustc 1.94.1 runs its front end, this codegen
backend converts each mono item's MIR to a kfrc Typed Body, builds and
verifies a monomorphic SoN graph (`kfrc-son`), lowers it to x86-64
(`kfrc-x86`), and hands one object per codegen unit back to rustc's linker.

Rust semantic values are the sole production IR. Every function goes through
`build_graph → verified semantic optimization → explicit layout lowering → x86`.
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
Object function/data order and CGU-local inlining restrictions are preserved.
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
