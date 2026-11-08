#!/bin/sh
# Generative differential: for each seed, generate a program, build the
# library with kfrc and with LLVM, link both against the LLVM-built driver,
# run, and compare. Stops at the first mismatch or kfrc failure.
# Usage: run.sh <librustc_codegen_kfrc.so> <work dir> <first seed> <last seed> [functions per program]
set -eu
backend=$1
work=$2
first=$3
last=$4
count=${5:-40}
here=$(cd "$(dirname "$0")" && pwd)
export RUSTC_BOOTSTRAP=1
rustc="rustc +${KFRC_TOOLCHAIN:-1.94.1}"
flags="--edition 2021 -Cpanic=abort -Coverflow-checks=off -Copt-level=0 -Cdebug-assertions=off -Zmir-opt-level=0 -Awarnings"
pass=0
seed=$first
while [ "$seed" -le "$last" ]; do
    dir="$work/seed-$seed"
    mkdir -p "$dir/kfrc" "$dir/llvm"
    python3 -I "$here/generate.py" "$seed" "$count" "$dir"
    $rustc $flags --crate-type rlib --crate-name generated "$dir/lib.rs" --out-dir "$dir/llvm"
    if ! $rustc $flags --crate-type rlib --crate-name generated -Zcodegen-backend="$backend" "$dir/lib.rs" --out-dir "$dir/kfrc" 2> "$dir/kfrc-build.log"; then
        echo "seed $seed: kfrc build failed"; tail -5 "$dir/kfrc-build.log"; exit 1
    fi
    for arm in kfrc llvm; do
        $rustc $flags --crate-name driver "$dir/driver.rs" --extern generated="$dir/$arm/libgenerated.rlib" -o "$dir/$arm/driver"
        timeout 20 "$dir/$arm/driver" > "$dir/$arm.out" || { echo "seed $seed: $arm driver failed or timed out"; exit 1; }
    done
    if ! cmp -s "$dir/kfrc.out" "$dir/llvm.out"; then
        echo "seed $seed: MISMATCH"; diff "$dir/llvm.out" "$dir/kfrc.out" | head -10; exit 1
    fi
    pass=$((pass + 1))
    seed=$((seed + 1))
done
echo "PASS: $pass programs x $count functions, outputs identical"
