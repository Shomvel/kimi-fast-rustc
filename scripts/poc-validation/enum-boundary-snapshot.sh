#!/usr/bin/env bash
set -euo pipefail
backend=$(realpath "${1:?backend dylib}")
work=${2:?fresh result directory}
here=$(cd "$(dirname "$0")/fixtures" && pwd)
mkdir "$work"
work=$(realpath "$work")
export RUSTC_BOOTSTRAP=1
flags=(--edition=2021 -Cpanic=abort -Copt-level=3 -Zmir-opt-level=0 -Ccodegen-units=16)
if [ -n "${KFRC_SYSROOT:-}" ]; then flags+=(--sysroot="$KFRC_SYSROOT"); fi
for arm in llvm kfrc; do
    mkdir "$work/$arm"
    backend_flags=()
    if [ "$arm" = kfrc ]; then backend_flags=(-Zcodegen-backend="$backend"); fi
    (
        set -x
        rustc +"${KFRC_TOOLCHAIN:-1.94.1}" "${flags[@]}" "${backend_flags[@]}" --crate-name=enum_boundary_snapshot --crate-type=rlib "$here/enum-boundary-snapshot.rs" --out-dir="$work/$arm"
        rustc +"${KFRC_TOOLCHAIN:-1.94.1}" "${flags[@]}" "$here/enum-boundary-snapshot-driver.rs" --extern="enum_boundary_snapshot=$work/$arm/libenum_boundary_snapshot.rlib" -o "$work/$arm/driver"
    ) > "$work/$arm/build.log" 2>&1
    "$work/$arm/driver" > "$work/$arm/output.txt" 2> "$work/$arm/stderr.txt"
done
cmp "$work/llvm/output.txt" "$work/kfrc/output.txt"
sha256sum "$backend" "$here/enum-boundary-snapshot.rs" "$here/enum-boundary-snapshot-driver.rs" "$work/llvm/driver" "$work/kfrc/driver" > "$work/sha256.txt"
cat "$work/kfrc/output.txt"
