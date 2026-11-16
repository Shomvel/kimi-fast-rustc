#!/usr/bin/env bash
set -euo pipefail
backend=$(realpath "${1:?backend dylib}")
work=${2:?result directory}
here=$(cd "$(dirname "$0")" && pwd)
toolchain=${KFRC_TOOLCHAIN:-1.94.1}
export RUSTC_BOOTSTRAP=1
mkdir -p "$work"
flags=(--edition=2021 -Cpanic=abort -Coverflow-checks=on -Cdebug-assertions=off -Zmir-opt-level=0)
for arm in cranelift x86 llvm; do
    mkdir -p "$work/$arm"
    if [ "$arm" = llvm ]; then
        rustc +"$toolchain" "${flags[@]}" --crate-type=rlib --crate-name=switches "$here/switches.rs" --out-dir="$work/$arm"
    else
        env KFRC_MACHINE="$arm" KFRC_VERIFY=full KFRC_CRANELIFT_VERIFY=1 KFRC_DUMP_GRAPH=1 KFRC_DUMP_MACHINE=switches \
            rustc +"$toolchain" "${flags[@]}" -Zcodegen-backend="$backend" --crate-type=rlib --crate-name=switches \
            "$here/switches.rs" --out-dir="$work/$arm" > "$work/$arm/compile.stdout" 2> "$work/$arm/compile.stderr"
        grep -q ' = Switch\[' "$work/$arm/compile.stderr"
    fi
    rustc +"$toolchain" "${flags[@]}" "$here/driver.rs" --extern=switches="$work/$arm/libswitches.rlib" -o "$work/$arm/driver"
    "$work/$arm/driver" > "$work/$arm/output.txt"
done
cmp "$work/cranelift/output.txt" "$work/llvm/output.txt"
cmp "$work/x86/output.txt" "$work/llvm/output.txt"
grep -q 'br_table ' "$work/cranelift/compile.stderr"
sha256sum "$here/switches.rs" "$here/driver.rs" "$backend" "$work"/*/libswitches.rlib "$work"/*/driver > "$work/sha256.txt"
printf 'PASS: integer switches through Cranelift and x86 match LLVM and the driver oracle\n'
