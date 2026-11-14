#!/usr/bin/env bash
set -euo pipefail
backend=$(realpath "${1:?backend dylib}")
work=${2:?fresh result directory}
script=$(realpath "$0")
here=$(cd "$(dirname "$script")/fixtures" && pwd)
mkdir "$work"
work=$(realpath "$work")
mkdir "$work/inputs"
cp "$script" "$work/inputs/context-inline.sh"
cp "$here/context-inline.rs" "$here/context-inline-driver.rs" "$work/inputs/"
export RUSTC_BOOTSTRAP=1
toolchain=${KFRC_TOOLCHAIN:-1.94.1}
flags=(--edition=2021 -Cpanic=abort -Copt-level=3 -Zmir-opt-level=0 -Ccodegen-units=16)
if [ -n "${KFRC_SYSROOT:-}" ]; then flags+=(--sysroot="$KFRC_SYSROOT"); fi

# Keep all caller gates. KFRC_CONTEXT_INLINE=1 is experimental opt-in;
# unset/0 retains the source behavior and none is overridden here.
{
    printf 'toolchain=%s\n' "$toolchain"
    printf 'sysroot=%s\n' "${KFRC_SYSROOT:-toolchain-default}"
    printf 'KFRC_CONTEXT_INLINE=%s\n' "${KFRC_CONTEXT_INLINE:-unset}"
    printf 'KFRC_SPECIALIZATION_STATS=%s\n' "${KFRC_SPECIALIZATION_STATS:-unset}"
    printf 'KFRC_LAYOUT_CALL_CONSTANTS=%s\n' "${KFRC_LAYOUT_CALL_CONSTANTS:-unset}"
    printf 'KFRC_LAYOUT_CALL_CONSTANTS_STATS=%s\n' "${KFRC_LAYOUT_CALL_CONSTANTS_STATS:-unset}"
    printf 'corpus=context-inline-v1; seed=ca711face01d5eed; literal_sites=40; random_strings=512; loop_sequences=256; loop_steps=48\n'
} > "$work/inputs/configuration.txt"
rustc +"$toolchain" -vV > "$work/inputs/toolchain.txt"
uname -a > "$work/inputs/host.txt"
compiler_sysroot=$(rustc +"$toolchain" --print=sysroot)
effective_sysroot=$(rustc +"$toolchain" "${flags[@]}" --print=sysroot)
target=$(awk '/^host: / { print $2 }' "$work/inputs/toolchain.txt")
sha256sum "$backend" "$compiler_sysroot/bin/rustc" "$work/inputs/"* > "$work/sha256.txt"
find "$effective_sysroot/lib/rustlib/$target/lib" -maxdepth 1 -type f \
    \( -name '*.rlib' -o -name '*.so' -o -name '*.dylib' -o -name '*.a' -o -name '*.o' \) \
    -print0 | sort -z | xargs -0 sha256sum > "$work/sysroot-sha256.txt"

# Each library links into an LLVM-built driver with independent byte loads.
for arm in llvm kfrc; do
    mkdir "$work/$arm"
    backend_flags=()
    if [ "$arm" = kfrc ]; then backend_flags=(-Zcodegen-backend="$backend"); fi
    (
        set -x
        rustc +"$toolchain" "${flags[@]}" "${backend_flags[@]}" \
            --crate-name=context_inline --crate-type=rlib \
            "$work/inputs/context-inline.rs" --out-dir="$work/$arm"
        rustc +"$toolchain" "${flags[@]}" "$work/inputs/context-inline-driver.rs" \
            --extern="context_inline=$work/$arm/libcontext_inline.rlib" -o "$work/$arm/driver"
    ) > "$work/$arm/build.log" 2>&1
    awk '/^kfrc (inlining|preparation|specialization|inline preparation)|^kfrc-context|^kfrc-layout-call-constants: |^\[kfrc-instcombine\]/' \
        "$work/$arm/build.log" > "$work/$arm/context-inline-stats.txt"
    "$work/$arm/driver" "$work/$arm/inputs.bin" \
        > "$work/$arm/output.txt" 2> "$work/$arm/stderr.txt"
    sha256sum "$work/$arm/libcontext_inline.rlib" "$work/$arm/driver" \
        "$work/$arm/inputs.bin" >> "$work/sha256.txt"
    if command -v nm >/dev/null 2>&1; then
        nm -A "$work/$arm/libcontext_inline.rlib" "$work/$arm/driver" \
            > "$work/$arm/symbols.txt" 2> "$work/$arm/nm.stderr"
    fi
    if command -v objdump >/dev/null 2>&1; then
        objdump -dr -Mintel "$work/$arm/libcontext_inline.rlib" "$work/$arm/driver" \
            > "$work/$arm/disassembly.txt" 2> "$work/$arm/objdump.stderr"
    fi
    if [ "$arm" != llvm ]; then
        cmp "$work/llvm/inputs.bin" "$work/$arm/inputs.bin"
        cmp "$work/llvm/output.txt" "$work/$arm/output.txt"
    fi
done
cat "$work/kfrc/output.txt"
