#!/usr/bin/env bash
# A real rustc/library/LLVM-driver differential, including raw partial init.
set -euo pipefail
backend=$(realpath "${1:?backend library}")
work=${2:?fresh output directory}
here=$(cd "$(dirname "$0")" && pwd)
toolchain=${KFRC_TOOLCHAIN:-1.94.1}
if [ -e "$work" ]; then
    printf 'result directory already exists: %s\n' "$work" >&2
    exit 2
fi
mkdir -p "$work"
work=$(realpath "$work")
flags=(--edition=2021 -Cpanic=abort -Coverflow-checks=off -Copt-level=3 -Ccodegen-units=1 -Zmir-opt-level=0)
clean=(env -u KFRC_OPTIMIZE -u KFRC_INLINE -u KFRC_CRANELIFT_PROMOTE RUSTC_BOOTSTRAP=1)
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
for arm in llvm x86 cranelift-memory cranelift-lanes; do
    mkdir "$work/$arm"
    compiler=("${clean[@]}")
    backend_flags=()
    case "$arm" in
        x86) compiler+=(KFRC_MACHINE=x86); backend_flags=(-Zcodegen-backend="$backend");;
        cranelift-memory) compiler+=(KFRC_MACHINE=cranelift KFRC_CRANELIFT_VERIFY=1 KFRC_CRANELIFT_PROMOTE=0); backend_flags=(-Zcodegen-backend="$backend");;
        cranelift-lanes) compiler+=(KFRC_MACHINE=cranelift KFRC_CRANELIFT_VERIFY=1 KFRC_CRANELIFT_PROMOTE=1 KFRC_CRANELIFT_STORAGE_REPORT=1); backend_flags=(-Zcodegen-backend="$backend");;
    esac
    run "$arm-library" "${compiler[@]}" rustc +"$toolchain" "${flags[@]}" "${backend_flags[@]}" \
        --crate-type=rlib --crate-name=storage_representation "$here/library.rs" --out-dir="$work/$arm"
    run "$arm-driver" "${clean[@]}" rustc +"$toolchain" "${flags[@]}" "$here/driver.rs" \
        --extern=storage_representation="$work/$arm/libstorage_representation.rlib" -o "$work/$arm/driver"
    run "$arm-execute" "$work/$arm/driver"
    cmp "$work/llvm-execute.stdout" "$work/$arm-execute.stdout"
    sha256sum "$work/$arm/driver" "$work/$arm/libstorage_representation.rlib" > "$work/$arm/outputs.sha256"
done
printf 'All four storage representation arms agree.\n'
