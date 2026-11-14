#!/usr/bin/env bash
# Verify library behavior with the same LLVM-compiled caller in each arm.
set -euo pipefail
backend=$(realpath "${1:?candidate backend}")
work=${2:?fresh output directory}
control=${3:-}
here=$(cd "$(dirname "$0")" && pwd)
toolchain=${KFRC_TOOLCHAIN:-1.94.1}
test ! -e "$work"
mkdir -p "$work"
work=$(realpath "$work")
flags=(--edition=2021 -Cpanic=abort -Coverflow-checks=off -Copt-level=3 -Ccodegen-units=1 -Zmir-opt-level=0)
clean=(env -u KFRC_OPTIMIZE -u KFRC_INLINE RUSTC_BOOTSTRAP=1)
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
arms=(llvm candidate)
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
    run "$arm-library" "${clean[@]}" KFRC_MACHINE=cranelift KFRC_CRANELIFT_VERIFY=1 \
        rustc +"$toolchain" "${flags[@]}" "${backend_flags[@]}" \
        --crate-type=rlib --crate-name=static_loads "$here/library.rs" --out-dir="$work/$arm"
    run "$arm-driver" "${clean[@]}" rustc +"$toolchain" "${flags[@]}" "$here/driver.rs" \
        --extern=static_loads="$work/$arm/libstatic_loads.rlib" -o "$work/$arm/driver"
    run "$arm-execute" "$work/$arm/driver"
    cmp "$work/llvm-execute.stdout" "$work/$arm-execute.stdout"
done
printf 'All static-load arms agree.\n'
