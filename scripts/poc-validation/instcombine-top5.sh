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

# The kfrc rlib links into an LLVM-built driver with independent references.
# Keep the caller's rule gates and diagnostics unchanged so the validation
# runner can use this same entry for default, all-off and leave-one-out arms.
for arm in llvm kfrc; do
    mkdir "$work/$arm"
    backend_flags=()
    if [ "$arm" = kfrc ]; then backend_flags=(-Zcodegen-backend="$backend"); fi
    (
        set -x
        rustc +"${KFRC_TOOLCHAIN:-1.94.1}" "${flags[@]}" "${backend_flags[@]}" --crate-name=instcombine_top5 --crate-type=rlib "$here/instcombine-top5.rs" --out-dir="$work/$arm"
        rustc +"${KFRC_TOOLCHAIN:-1.94.1}" "${flags[@]}" "$here/instcombine-top5-driver.rs" --extern="instcombine_top5=$work/$arm/libinstcombine_top5.rlib" -o "$work/$arm/driver"
    ) > "$work/$arm/build.log" 2>&1
    "$work/$arm/driver" > "$work/$arm/output.txt" 2> "$work/$arm/stderr.txt"
    if [ "$arm" != llvm ]; then cmp "$work/llvm/output.txt" "$work/$arm/output.txt"; fi
    {
        printf 'disabled=%s\n' "${KFRC_INSTCOMBINE_DISABLE:-}"
        printf 'stats=%s\n' "${KFRC_INSTCOMBINE_STATS:-}"
        for rule in mul-pow2 trunc-binop opposite-shifts mul-shift-factor memcmp-zero; do
            count=$(awk -v rule="rule=$rule" '$0 ~ /\[kfrc-instcombine\]/ && $2 == rule { n++ } END { print n+0 }' "$work/$arm/build.log")
            printf '%s=%s\n' "$rule" "$count"
        done
    } > "$work/$arm/rules.txt"
    if command -v nm >/dev/null 2>&1; then nm -A "$work/$arm/libinstcombine_top5.rlib" > "$work/$arm/symbols.txt" 2> "$work/$arm/nm.stderr"; fi
    if command -v objdump >/dev/null 2>&1; then objdump -dr "$work/$arm/libinstcombine_top5.rlib" > "$work/$arm/disassembly.txt" 2> "$work/$arm/objdump.stderr"; fi
done
sha256sum "$backend" "$here/instcombine-top5.rs" "$here/instcombine-top5-driver.rs" "$0" > "$work/sha256.txt"
for arm in llvm kfrc; do
    sha256sum "$work/$arm/libinstcombine_top5.rlib" "$work/$arm/driver" >> "$work/sha256.txt"
done
cat "$work/kfrc/output.txt"
