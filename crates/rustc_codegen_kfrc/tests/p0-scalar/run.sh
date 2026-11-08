#!/bin/sh
# P0 end-to-end: build `scalar` with kfrc and with LLVM, link both against the
# same LLVM-built driver, run, and compare outputs byte for byte.
# Usage: run.sh <path to librustc_codegen_kfrc.so> <work dir>
set -eu
backend=$1
work=$2
here=$(cd "$(dirname "$0")" && pwd)
toolchain=${KFRC_TOOLCHAIN:-1.94.1}
rustc="rustc +$toolchain"
export RUSTC_BOOTSTRAP=1
flags="--edition 2021 -Cpanic=abort -Coverflow-checks=off -Copt-level=0 -Cdebug-assertions=off -Zmir-opt-level=0"
mkdir -p "$work/kfrc" "$work/llvm"
$rustc $flags --crate-type rlib --crate-name scalar -Zcodegen-backend="$backend" "$here/scalar.rs" --out-dir "$work/kfrc"
$rustc $flags --crate-type rlib --crate-name scalar "$here/scalar.rs" --out-dir "$work/llvm"
for arm in kfrc llvm; do
    $rustc $flags --crate-name driver "$here/driver.rs" --extern scalar="$work/$arm/libscalar.rlib" -o "$work/$arm/driver"
    "$work/$arm/driver" > "$work/$arm.out"
done
# Provenance: the kfrc rlib's object must come from kfrc's writer.
mkdir -p "$work/kfrc/objects"
(cd "$work/kfrc/objects" && ar x ../libscalar.rlib)
readelf -s "$work"/kfrc/objects/*.o | grep -q 'FILE.*kfrc' && echo "provenance: scalar object written by kfrc"
sha256sum "$work"/kfrc/objects/*.o "$work/kfrc/driver" "$work/llvm/driver"
lines=$(wc -l < "$work/llvm.out")
if cmp -s "$work/kfrc.out" "$work/llvm.out"; then
    echo "PASS: $lines lines identical"
else
    diff "$work/llvm.out" "$work/kfrc.out" | head -40
    echo "FAIL"
    exit 1
fi
