#!/usr/bin/env bash
# x86_64-unknown-linux-gnu, Rust 1.94.1. Each arm uses one shared LLVM
# dependency, driver and assembly source. No timing is performed here.
set -euo pipefail
backend=$(realpath "${1:?candidate backend}")
work=${2:?fresh output directory}
control=${3:-}
here=$(cd "$(dirname "$0")" && pwd)
toolchain=${KFRC_TOOLCHAIN:-1.94.1}
test "$(uname -sm)" = 'Linux x86_64'
test ! -e "$work"
mkdir -p "$work"
work=$(realpath "$work")
flags=(--edition=2021 -Cpanic=abort -Coverflow-checks=off -Copt-level=3 -Ccodegen-units=1 -Zmir-opt-level=0)
clean=(env -u KFRC_OPTIMIZE -u KFRC_INLINE -u KFRC_SCALAR_ABI RUSTC_BOOTSTRAP=1)
run() {
    local label=$1 status=0
    shift
    printf '%q ' "$@" > "$work/$label.command"
    printf '\n' >> "$work/$label.command"
    "$@" > "$work/$label.stdout" 2> "$work/$label.stderr" || status=$?
    printf '{"exit_code":%s}\n' "$status" > "$work/$label.result.json"
    if [ "$status" != 0 ]; then
        cat "$work/$label.stderr" >&2
        return "$status"
    fi
}
rustc +"$toolchain" -Vv > "$work/toolchain.txt"
cc --version > "$work/cc-version.txt"
sha256sum "$backend" "$here/foreign.rs" "$here/library.rs" "$here/driver.rs" "$here/shims.S" > "$work/inputs.sha256"
run shims cc -c "$here/shims.S" -o "$work/shims.o"
run foreign "${clean[@]}" rustc +"$toolchain" "${flags[@]}" --crate-type=rlib \
    --crate-name=scalar_abi_foreign "$here/foreign.rs" --out-dir="$work"
arms=(llvm candidate1 candidate4 candidate-x86 legacy)
if [ -n "$control" ]; then
    control=$(realpath "$control")
    sha256sum "$control" >> "$work/inputs.sha256"
    arms+=(control)
fi
for arm in "${arms[@]}"; do
    mkdir "$work/$arm"
    backend_flags=()
    config=()
    if [ "$arm" = llvm ]; then backend_flags=(--emit=link,llvm-ir); fi
    if [ "$arm" != llvm ]; then
        selected=$backend
        if [ "$arm" = control ]; then selected=$control; fi
        backend_flags=(-Zcodegen-backend="$selected")
        config=(KFRC_MACHINE=cranelift KFRC_VERIFY=full KFRC_CRANELIFT_VERIFY=1 KFRC_CODEGEN_THREADS=1)
        if [ "$arm" = candidate4 ]; then config+=(KFRC_CODEGEN_THREADS=4); fi
        if [ "$arm" = candidate-x86 ]; then config+=(KFRC_MACHINE=x86); fi
        if [ "$arm" = legacy ]; then config+=(KFRC_SCALAR_ABI=legacy); fi
    fi
    run "$arm-library" "${clean[@]}" "${config[@]}" rustc +"$toolchain" "${flags[@]}" "${backend_flags[@]}" \
        --crate-type=rlib --crate-name=scalar_abi_candidate "$here/library.rs" \
        --extern=scalar_abi_foreign="$work/libscalar_abi_foreign.rlib" -L dependency="$work" --out-dir="$work/$arm"
    run "$arm-driver" "${clean[@]}" rustc +"$toolchain" "${flags[@]}" "$here/driver.rs" \
        --extern=scalar_abi_candidate="$work/$arm/libscalar_abi_candidate.rlib" \
        --extern=scalar_abi_foreign="$work/libscalar_abi_foreign.rlib" -L dependency="$work" \
        -C link-arg="$work/shims.o" -o "$work/$arm/driver"
    run "$arm-execute" "$work/$arm/driver"
    cmp "$work/llvm-execute.stdout" "$work/$arm-execute.stdout"
    # This strict regression is separate: the previous producer discards
    # signext and treats transparent aggregate bytes as an unsigned scalar.
    # Run only positive oracles here. Inspect legacy/control once separately
    # and retain their actual exit status; never call them a passing oracle.
    if [ "$arm" != legacy ] && [ "$arm" != control ]; then
        run "$arm-wrapper-extension" "$work/$arm/driver" --wrapper-extension
        cmp "$work/llvm-wrapper-extension.stdout" "$work/$arm-wrapper-extension.stdout"
    fi
done
sha256sum "$work/shims.o" "$work/libscalar_abi_foreign.rlib" "$work"/*/libscalar_abi_candidate.rlib \
    "$work"/*/driver > "$work/outputs.sha256"
printf 'All scalar ABI matrix arms agree.\n'
