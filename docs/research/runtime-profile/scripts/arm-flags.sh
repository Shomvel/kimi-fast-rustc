#!/usr/bin/env bash
# Attribution arms: one place that defines what each arm changes.
#
# usage: arm-flags.sh --list        arm names in build order
#        arm-flags.sh --kind ARM    "rustflags" or "patch" (S arms rebuild with patched crates)
#        arm-flags.sh ARM           RUSTFLAGS for the arm (patch arms: none)
#
# Every arm changes one thing relative to A (stock `cargo build --release`), except C,
# which stacks the backend gaps a first kfrc backend is expected to have (V+U+R+J).
# LLVM 21.1.8 has no -opt-disable, so G/H switch off sub-features (see ../attribution.md).
# The prebuilt std is LLVM-O3 in every arm (flags apply only to crates Cargo compiles).
set -euo pipefail
LIST="A V U R F J I G H L O1 C S2 S0"
flags() {
    case $1 in
        A)  echo "" ;;
        V)  echo "-Cno-vectorize-loops -Cno-vectorize-slp" ;;                       # auto-vectorization off
        U)  echo "-Cllvm-args=-unroll-count=1 -Cllvm-args=-unroll-allow-peeling=false" ;;  # loop unroll + peel off
        R)  echo "-Cllvm-args=-regalloc=basic -Cllvm-args=-optimize-regalloc" ;;     # basic allocator; -optimize-regalloc lets the O0 allocator-shim module accept it
        F)  echo "-Cllvm-args=-regalloc=fast" ;;                                     # local single-pass allocator
        J)  echo "-Cllvm-args=-min-jump-table-entries=100000" ;;                     # no jump tables
        I)  echo "-Cllvm-args=-inline-threshold=0" ;;                                # LLVM inliner only for free calls
        G)  echo "-Cllvm-args=-enable-gvn-memdep=false -Cllvm-args=-enable-load-pre=false -Cllvm-args=-enable-pre=false" ;;  # GVN load elim + PRE off
        H)  echo "-Cllvm-args=-disable-licm-promotion" ;;                             # LICM scalar promotion off (hoisting stays)
        L)  echo "-Clto=off" ;;                                                      # no cross-CGU (ThinLocal) inlining
        O1) echo "-Copt-level=1" ;;                                                  # simple LLVM pipeline
        C)  echo "$(flags V) $(flags U) $(flags R) $(flags J)" ;;
        S2|S0) echo "" ;;
        *) echo "unknown arm $1" >&2; exit 2 ;;
    esac
}
case ${1:?usage} in
    --list) echo $LIST ;;
    --kind) case $2 in S2|S0) echo patch ;; *) flags "$2" >/dev/null; echo rustflags ;; esac ;;
    *) flags "$1" ;;
esac
