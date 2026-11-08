# rustc_codegen_kfrc

PoC shell (`docs/plan/poc/`): rustc 1.94.1 runs its front end, this codegen
backend converts each mono item's MIR to a kfrc Typed Body, builds and
verifies an R-level SoN graph (`kfrc-son`), lowers it to x86-64
(`kfrc-x86`), and hands one object per codegen unit back to rustc's linker.

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
```

Debugging switches: `KFRC_DUMP_BODY=1`, `KFRC_DUMP_GRAPH=1`,
`KFRC_DUMP_MACHINE=<symbol substring>`, and
`KFRC_INTERPRET=<symbol substring>=<arg>,<arg>` (runs the reference
interpreter on the verified graph, to tell graph bugs from lowering bugs).
`tests/p0-scalar/generated/reduce.py` delta-reduces a mismatching
generated function.

Unsupported input is a fatal error naming the function and construct; see
`docs/plan/poc/p0-status.md` for the supported subset.
