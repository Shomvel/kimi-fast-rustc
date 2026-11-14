#!/usr/bin/env bash
# Concrete -> dyn -> LLVM formatting and LLVM -> dyn -> kfrc dispatch probes.
set -euo pipefail
backend=$(realpath "${1:?backend dylib}")
work=${2:?fresh result directory}
here=$(cd "$(dirname "$0")" && pwd)
toolchain=${KFRC_TOOLCHAIN:-1.94.1}
export RUSTC_BOOTSTRAP=1
mkdir "$work"
work=$(realpath "$work")
rustc +"$toolchain" -vV > "$work/rustc-version.txt"
sha256sum "$backend" "$here/dyn-probe.rs" "$here/dyn-probe-driver.rs" "$here/dyn-probe.sh" > "$work/input.sha256"
flags=(--edition=2021 -Cpanic=abort -Copt-level=0 -Coverflow-checks=off -Cdebug-assertions=off -Zmir-opt-level=0)
for arm in llvm kfrc; do
    mkdir "$work/$arm"
    backend_flags=()
    if [ "$arm" = kfrc ]; then backend_flags=(-Zcodegen-backend="$backend"); fi
    (
        set -x
        rustc +"$toolchain" "${flags[@]}" "${backend_flags[@]}" \
            --crate-type=rlib --crate-name=dyn_probe "$here/dyn-probe.rs" \
            --out-dir="$work/$arm"
        rustc +"$toolchain" "${flags[@]}" "$here/dyn-probe-driver.rs" \
            --extern="dyn_probe=$work/$arm/libdyn_probe.rlib" -o "$work/$arm/driver"
    ) > "$work/$arm/build.log" 2>&1
    "$work/$arm/driver" > "$work/$arm/output.txt" 2> "$work/$arm/stderr.txt"
    sha256sum "$work/$arm/libdyn_probe.rlib" "$work/$arm/driver" > "$work/$arm/output.sha256"
done
mkdir "$work/kfrc/objects"
(cd "$work/kfrc/objects" && ar x ../libdyn_probe.rlib)
readelf -Ws "$work/kfrc/objects"/*.o > "$work/kfrc/object-symbols.txt"
grep -q 'FILE.*kfrc' "$work/kfrc/object-symbols.txt"
cmp "$work/llvm/output.txt" "$work/kfrc/output.txt"
printf 'PASS: %s dyn/derive/closure output lines identical\n' "$(wc -l < "$work/llvm/output.txt")"
