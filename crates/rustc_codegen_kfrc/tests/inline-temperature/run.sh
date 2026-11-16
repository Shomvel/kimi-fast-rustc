#!/usr/bin/env bash
# Verify library behavior with the same LLVM-compiled caller in each arm.
set -euo pipefail
ulimit -c 0
backend=$(realpath "${1:?candidate backend}")
work=${2:?fresh output directory}
control=${3:-}
here=$(cd "$(dirname "$0")" && pwd)
toolchain=${KFRC_TOOLCHAIN:-1.94.1}
test ! -e "$work"
mkdir -p "$work"
work=$(realpath "$work")
flags=(--edition=2021 -Cpanic=abort -Coverflow-checks=off -Copt-level=3 -Ccodegen-units=1 -Zmir-opt-level=0)
clean=(env -u KFRC_OPTIMIZE -u KFRC_INLINE -u KFRC_INLINE_COST_MODEL -u KFRC_INLINE_RESIDUAL_UNITS RUSTC_BOOTSTRAP=1)
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
sha256sum "$backend" "$here/library.rs" "$here/driver.rs" > "$work/inputs.sha256"
arms=(llvm candidate uniform32)
if [ -n "$control" ]; then
    control=$(realpath "$control")
    sha256sum "$control" >> "$work/inputs.sha256"
    arms+=(control)
fi
for arm in "${arms[@]}"; do
    mkdir "$work/$arm"
    backend_flags=()
    if [ "$arm" != llvm ]; then
        selected=$backend
        if [ "$arm" = control ]; then selected=$control; fi
        backend_flags=(-Zcodegen-backend="$selected")
    fi
    policy=()
    if [ "$arm" = uniform32 ]; then policy=(KFRC_INLINE_COST_MODEL=residual KFRC_INLINE_RESIDUAL_UNITS=32); fi
    run "$arm-library" "${clean[@]}" "${policy[@]}" KFRC_MACHINE=cranelift KFRC_CRANELIFT_VERIFY=1 \
        rustc +"$toolchain" "${flags[@]}" "${backend_flags[@]}" \
        --crate-type=rlib --crate-name=inline_temperature "$here/library.rs" --out-dir="$work/$arm"
    run "$arm-driver" "${clean[@]}" rustc +"$toolchain" "${flags[@]}" "$here/driver.rs" \
        --extern=inline_temperature="$work/$arm/libinline_temperature.rlib" -o "$work/$arm/driver"
    run "$arm-execute" "$work/$arm/driver"
    cmp "$work/llvm-execute.stdout" "$work/$arm-execute.stdout"
    # The panic-only source path must retain its observable message and abort.
    # Record the expected failure separately from the successful checksum run.
    status=0
    run "$arm-cold" "$work/$arm/driver" cold || status=$?
    test "$status" -eq 134
    grep -q '^cold value 0$' "$work/$arm-cold.stderr"
    cmp "$work/llvm-cold.stdout" "$work/$arm-cold.stdout"
    # Rust's default panic header includes the OS thread id. Keep the raw
    # stderr and normalize only that nondeterministic header field.
    sed -E "s/^(thread 'main') \([0-9]+\) panicked at /\1 panicked at /" \
        "$work/$arm-cold.stderr" > "$work/$arm-cold.normalized"
    grep -q "^thread 'main' panicked at .*library.rs:[0-9][0-9]*:[0-9][0-9]*:$" \
        "$work/$arm-cold.normalized"
    cmp "$work/llvm-cold.normalized" "$work/$arm-cold.normalized"
    status=0
    run "$arm-cold-abi" "$work/$arm/driver" cold-abi || status=$?
    test "$status" -eq 134
    grep -q '^cold ABI [0-9a-f]\{16\}$' "$work/$arm-cold-abi.stderr"
    sed -E "s/^(thread 'main') \([0-9]+\) panicked at /\1 panicked at /" \
        "$work/$arm-cold-abi.stderr" > "$work/$arm-cold-abi.normalized"
    cmp "$work/llvm-cold-abi.normalized" "$work/$arm-cold-abi.normalized"
done
printf 'All inline-temperature arms agree.\n'
