#!/usr/bin/env bash
# Same source, same LLVM ABI provider and driver, three library compilation arms.
set -euo pipefail
backend=$(realpath "${1:?backend dylib}")
work=${2:?fresh result directory}
here=$(cd "$(dirname "$0")" && pwd)
toolchain=${KFRC_TOOLCHAIN:-1.94.1}
if [ -e "$work" ]; then
    printf 'result directory already exists: %s\n' "$work" >&2
    exit 2
fi
mkdir -p "$work"
work=$(realpath "$work")
flags=(--edition=2021 -Cpanic=abort -Coverflow-checks=off -Cdebug-assertions=off -Zmir-opt-level=0 -Copt-level=0 -Ccodegen-units=1)
clean_env=(env -u KFRC_OPTIMIZE -u KFRC_INLINE -u KFRC_PROFILE -u KFRC_PROFILE_TOTAL -u KFRC_LLVM_PROFILE -u KFRC_SPECIALIZATION_STATS -u KFRC_DUMP_GRAPH -u KFRC_DUMP_BODY -u KFRC_INTERPRET RUSTC_BOOTSTRAP=1)
run() {
    local label=$1
    shift
    local status=0
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
sha256sum "$here/abi_reference.rs" "$here/values.rs" "$here/driver.rs" "$here/dynamic.rs" "$here"/support_*.rs "$here"/reject_*.rs "$backend" > "$work/inputs.sha256"
mkdir "$work/reference"
run provider "${clean_env[@]}" rustc +"$toolchain" "${flags[@]}" --crate-type=rlib --crate-name=abi_reference "$here/abi_reference.rs" --out-dir="$work/reference"
for arm in llvm semantic-off semantic-on; do
    mkdir "$work/$arm"
    compiler=("${clean_env[@]}")
    backend_flags=()
    case "$arm" in
        semantic-off) compiler+=(KFRC_OPTIMIZE=0 KFRC_DUMP_GRAPH=1); backend_flags=(-Zcodegen-backend="$backend");;
        semantic-on) compiler+=(KFRC_OPTIMIZE=1 KFRC_DUMP_GRAPH=1); backend_flags=(-Zcodegen-backend="$backend");;
    esac
    run "$arm-library" "${compiler[@]}" rustc +"$toolchain" "${flags[@]}" "${backend_flags[@]}" --crate-type=rlib --crate-name=semantic_values "$here/values.rs" --extern=abi_reference="$work/reference/libabi_reference.rlib" --out-dir="$work/$arm"
    run "$arm-driver-build" "${clean_env[@]}" rustc +"$toolchain" "${flags[@]}" "$here/driver.rs" --extern=semantic_values="$work/$arm/libsemantic_values.rlib" --extern=abi_reference="$work/reference/libabi_reference.rlib" -L dependency="$work/reference" -o "$work/$arm/driver"
    run "$arm-execute" "$work/$arm/driver"
    cmp "$work/llvm-execute.stdout" "$work/$arm-execute.stdout"
    mkdir "$work/$arm/objects"
    (cd "$work/$arm/objects" && ar x ../libsemantic_values.rlib)
    readelf -Ws "$work/$arm/objects"/*.o > "$work/$arm/object-symbols.txt"
    sha256sum "$work/$arm/objects"/*.o "$work/$arm/driver" > "$work/$arm/output.sha256"
done
for probe in union tls simd; do
    mkdir "$work/reject-$probe"
    status=0
    "${clean_env[@]}" rustc +"$toolchain" "${flags[@]}" -Zcodegen-backend="$backend" --crate-type=rlib --crate-name="reject_$probe" "$here/reject_$probe.rs" --out-dir="$work/reject-$probe" > "$work/reject-$probe.stdout" 2> "$work/reject-$probe.stderr" || status=$?
    printf '{"exit_code":%s}\n' "$status" > "$work/reject-$probe.result.json"
    if [ "$status" = 0 ] || [ -f "$work/reject-$probe/libreject_$probe.rlib" ]; then
        printf 'expected whole-crate rejection for %s\n' "$probe" >&2
        exit 1
    fi
done
python3 "$here/summarize.py" "$work"
