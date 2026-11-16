#!/usr/bin/env bash
set -euo pipefail
backend=$(realpath "${1:?backend dylib}")
work=${2:?fresh result directory}
script=$(realpath "$0")
here=$(cd "$(dirname "$script")/fixtures" && pwd)
mkdir "$work"
work=$(realpath "$work")
mkdir "$work/inputs"
cp "$script" "$work/inputs/layout-call-constants.sh"
cp "$here/layout-call-constants.rs" "$here/layout-call-constants-driver.rs" "$work/inputs/"
export RUSTC_BOOTSTRAP=1
toolchain=${KFRC_TOOLCHAIN:-1.94.1}
flags=(--edition=2021 -Cpanic=abort -Copt-level=3 -Zmir-opt-level=0 -Ccodegen-units=16)
if [ -n "${KFRC_SYSROOT:-}" ]; then flags+=(--sysroot="$KFRC_SYSROOT"); fi

# Preserve every caller gate, including KFRC_LAYOUT_CALL_CONSTANTS=0 and the
# optional KFRC_LAYOUT_CALL_CONSTANTS_STATS=1. Diagnostics stay in build.log.
{
    printf 'toolchain=%s\n' "$toolchain"
    printf 'sysroot=%s\n' "${KFRC_SYSROOT:-toolchain-default}"
    printf 'KFRC_LAYOUT_CALL_CONSTANTS=%s\n' "${KFRC_LAYOUT_CALL_CONSTANTS:-unset}"
    printf 'KFRC_LAYOUT_CALL_CONSTANTS_STATS=%s\n' "${KFRC_LAYOUT_CALL_CONSTANTS_STATS:-unset}"
    printf 'KFRC_INSTCOMBINE_DISABLE=%s\n' "${KFRC_INSTCOMBINE_DISABLE:-unset}"
    printf 'KFRC_INSTCOMBINE_STATS=%s\n' "${KFRC_INSTCOMBINE_STATS:-unset}"
    printf 'corpus=deterministic-v1; seed=7f031aced09b6285; random_pairs=1024; loop_sequences=512; loop_steps=48\n'
} > "$work/inputs/configuration.txt"
rustc +"$toolchain" -vV > "$work/inputs/toolchain.txt"
uname -a > "$work/inputs/host.txt"
compiler_sysroot=$(rustc +"$toolchain" --print=sysroot)
effective_sysroot=$(rustc +"$toolchain" "${flags[@]}" --print=sysroot)
target=$(awk '/^host: / { print $2 }' "$work/inputs/toolchain.txt")
sha256sum "$backend" "$compiler_sysroot/bin/rustc" "$work/inputs/"* > "$work/sha256.txt"
# Record all Rust target-library binary inputs, including a supplied sharedstd
# sysroot, without copying or printing environment credentials.
find "$effective_sysroot/lib/rustlib/$target/lib" -maxdepth 1 -type f \
    \( -name '*.rlib' -o -name '*.so' -o -name '*.dylib' -o -name '*.a' -o -name '*.o' \) \
    -print0 | sort -z | xargs -0 sha256sum > "$work/sysroot-sha256.txt"

# Each rlib links into an LLVM-built driver using the bytewise reference.
# The driver also emits the exact input transcript, independent of addresses.
for arm in llvm kfrc; do
    mkdir "$work/$arm"
    backend_flags=()
    if [ "$arm" = kfrc ]; then backend_flags=(-Zcodegen-backend="$backend"); fi
    (
        set -x
        rustc +"$toolchain" "${flags[@]}" "${backend_flags[@]}" \
            --crate-name=layout_call_constants --crate-type=rlib \
            "$work/inputs/layout-call-constants.rs" --out-dir="$work/$arm"
        rustc +"$toolchain" "${flags[@]}" "$work/inputs/layout-call-constants-driver.rs" \
            --extern="layout_call_constants=$work/$arm/liblayout_call_constants.rlib" \
            -o "$work/$arm/driver"
    ) > "$work/$arm/build.log" 2>&1
    awk '/^kfrc-layout-call-constants: /' "$work/$arm/build.log" > "$work/$arm/call-constants-stats.txt"
    "$work/$arm/driver" "$work/$arm/inputs.bin" \
        > "$work/$arm/output.txt" 2> "$work/$arm/stderr.txt"
    sha256sum "$work/$arm/liblayout_call_constants.rlib" "$work/$arm/driver" \
        "$work/$arm/inputs.bin" >> "$work/sha256.txt"
    if command -v nm >/dev/null 2>&1; then
        nm -A "$work/$arm/liblayout_call_constants.rlib" "$work/$arm/driver" \
            > "$work/$arm/symbols.txt" 2> "$work/$arm/nm.stderr"
    fi
    if command -v objdump >/dev/null 2>&1; then
        objdump -dr -Mintel "$work/$arm/liblayout_call_constants.rlib" "$work/$arm/driver" \
            > "$work/$arm/disassembly.txt" 2> "$work/$arm/objdump.stderr"
    fi
    if [ "$arm" != llvm ]; then
        cmp "$work/llvm/inputs.bin" "$work/$arm/inputs.bin"
        cmp "$work/llvm/output.txt" "$work/$arm/output.txt"
    fi
done
cat "$work/kfrc/output.txt"
