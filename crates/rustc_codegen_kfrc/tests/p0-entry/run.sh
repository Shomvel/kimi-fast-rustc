#!/bin/sh
# Executable end-to-end probes, each compared with the same LLVM toolchain.
# Usage: run.sh <backend.so> <work dir> [case names]
set -eu
backend=$1
work=$2
cases=${3:-"no_main main alloc_main alloc_fail_main"}
here=$(cd "$(dirname "$0")" && pwd)
toolchain=${KFRC_TOOLCHAIN:-1.94.1}
export RUSTC_BOOTSTRAP=1
mkdir -p "$work/kfrc" "$work/llvm"
for case_name in $cases; do
    for arm in llvm kfrc; do
        if [ "$arm" = kfrc ]; then
            set -- -Zcodegen-backend="$backend"
        else
            set --
        fi
        rustc "+$toolchain" --edition 2024 -Cpanic=abort -Coverflow-checks=off \
            -Copt-level=0 -Cdebug-assertions=off -Csave-temps -Zmir-opt-level=0 \
            --crate-name "$case_name" --emit=obj,link --out-dir "$work/$arm" \
            "$@" "$here/$case_name.rs"
        if "$work/$arm/$case_name" > "$work/$arm/$case_name.stdout" \
            2> "$work/$arm/$case_name.stderr"; then
            status=0
        else
            status=$?
        fi
        echo "$status" > "$work/$arm/$case_name.status"
    done
    cmp "$work/llvm/$case_name.status" "$work/kfrc/$case_name.status"
    cmp "$work/llvm/$case_name.stdout" "$work/kfrc/$case_name.stdout"
    cmp "$work/llvm/$case_name.stderr" "$work/kfrc/$case_name.stderr"
    case "$case_name" in
        no_main) expected_status=37;;
        alloc_fail_main) expected_status=134;;
        *) expected_status=0;;
    esac
    [ "$(cat "$work/kfrc/$case_name.status")" = "$expected_status" ]
    if [ "$case_name" = alloc_main ]; then
        grep -qx 'kfrc allocator Vec growth/drop passed' "$work/kfrc/$case_name.stdout"
    fi
    if [ "$case_name" = alloc_fail_main ]; then
        grep -q 'memory allocation of 16 bytes failed' "$work/kfrc/$case_name.stderr"
    fi
    provenance=no
    for object in "$work/kfrc/$case_name"*.o; do
        [ -f "$object" ] || continue
        if readelf -s "$object" | grep -q 'FILE.*kfrc'; then
            provenance=yes
        fi
        sha256sum "$object"
    done
    [ "$provenance" = yes ]
    sha256sum "$work/kfrc/$case_name" "$work/llvm/$case_name"
    echo "PASS: $case_name executable matches LLVM (exit $(cat "$work/kfrc/$case_name.status"))"
done
