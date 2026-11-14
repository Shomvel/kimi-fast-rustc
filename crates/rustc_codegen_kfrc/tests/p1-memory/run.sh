#!/usr/bin/env bash
set -euo pipefail
backend=$(realpath "${1:?backend dylib}")
work=${2:?result directory}
here=$(cd "$(dirname "$0")" && pwd)
toolchain=${KFRC_TOOLCHAIN:-1.94.1}
export RUSTC_BOOTSTRAP=1
mkdir -p "$work/kfrc" "$work/llvm"
flags=(--edition=2021 -Cpanic=abort -Coverflow-checks=on -Cdebug-assertions=off -Zmir-opt-level=0)
for arm in kfrc llvm; do
    backend_flags=()
    if [ "$arm" = kfrc ]; then backend_flags=(-Zcodegen-backend="$backend"); fi
    rustc +"$toolchain" "${flags[@]}" "${backend_flags[@]}" --crate-type=rlib --crate-name=memory "$here/memory.rs" --out-dir="$work/$arm"
    rustc +"$toolchain" "${flags[@]}" "$here/driver.rs" --extern=memory="$work/$arm/libmemory.rlib" -o "$work/$arm/driver"
    "$work/$arm/driver" > "$work/$arm.out"
done
cmp "$work/kfrc.out" "$work/llvm.out"
mkdir -p "$work/kfrc/objects"
(cd "$work/kfrc/objects" && ar x ../libmemory.rlib)
readelf -Ws "$work/kfrc/objects"/*.o > "$work/kfrc/object-symbols.txt"
grep -q 'FILE.*kfrc' "$work/kfrc/object-symbols.txt"
sha256sum "$work/kfrc/objects"/*.o "$work/kfrc/driver" "$work/llvm/driver" > "$work/output.sha256"
printf 'PASS: %s memory/aggregate output lines identical\n' "$(wc -l < "$work/llvm.out")"
for mode in division bounds overflow; do
    for arm in kfrc llvm; do
        set +e
        "$work/$arm/driver" "$mode" > "$work/$arm-$mode.stdout" 2> "$work/$arm-$mode.stderr"
        status=$?
        set -e
        printf '%s\n' "$status" > "$work/$arm-$mode.status"
        test "$status" -ne 0
    done
    cmp "$work/kfrc-$mode.status" "$work/llvm-$mode.status"
    case "$mode" in
        division) message='attempt to divide by zero';;
        bounds) message='index out of bounds';;
        overflow) message='attempt to add with overflow';;
    esac
    grep -q "$message" "$work/kfrc-$mode.stderr"
    sed -n 's/^.*panicked at \(.*memory\.rs:[0-9]*:[0-9]*\):$/\1/p' "$work/kfrc-$mode.stderr" > "$work/kfrc-$mode.location"
    sed -n 's/^.*panicked at \(.*memory\.rs:[0-9]*:[0-9]*\):$/\1/p' "$work/llvm-$mode.stderr" > "$work/llvm-$mode.location"
    test -s "$work/kfrc-$mode.location"
    cmp "$work/kfrc-$mode.location" "$work/llvm-$mode.location"
    printf 'PASS: %s abort status, panic message and exact source Location\n' "$mode"
done
