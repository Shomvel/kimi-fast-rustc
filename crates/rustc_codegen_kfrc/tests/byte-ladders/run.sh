#!/usr/bin/env bash
# One LLVM caller/oracle against candidate, disabled pass, and optional base.
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
clean=(env -u KFRC_OPTIMIZE -u KFRC_INLINE -u KFRC_LAYOUT_BYTE_LADDERS -u KFRC_DUMP_GRAPH RUSTC_BOOTSTRAP=1)
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
arms=(llvm candidate disabled)
if [ -n "$control" ]; then
    control=$(realpath "$control")
    sha256sum "$control" >> "$work/inputs.sha256"
    arms+=(control)
fi
for arm in "${arms[@]}"; do
    mkdir "$work/$arm"
    backend_flags=()
    extra=()
    if [ "$arm" != llvm ]; then
        selected=$backend
        if [ "$arm" = control ]; then selected=$control; fi
        backend_flags=(-Zcodegen-backend="$selected")
        extra=(KFRC_MACHINE=cranelift KFRC_CRANELIFT_VERIFY=1 KFRC_VERIFY=full
            KFRC_INLINE_RESIDUAL_UNITS=256 KFRC_LAYOUT_BYTE_LADDERS_STATS=1)
        if [ "$arm" = disabled ]; then extra+=(KFRC_LAYOUT_BYTE_LADDERS=0); fi
    fi
    run "$arm-library" "${clean[@]}" "${extra[@]}" rustc +"$toolchain" "${flags[@]}" "${backend_flags[@]}" \
        --crate-type=rlib --crate-name=byte_ladders "$here/library.rs" --out-dir="$work/$arm"
    run "$arm-driver" "${clean[@]}" rustc +"$toolchain" "${flags[@]}" "$here/driver.rs" \
        --extern=byte_ladders="$work/$arm/libbyte_ladders.rlib" -o "$work/$arm/driver"
    run "$arm-execute" "$work/$arm/driver"
    cmp "$work/llvm-execute.stdout" "$work/$arm-execute.stdout"
done
python3 - "$work/candidate-library.stderr" "$work/disabled-library.stderr" <<'PY'
from pathlib import Path
import re, sys
candidate = Path(sys.argv[1]).read_text()
disabled = Path(sys.argv[2]).read_text()
counts = [int(n) for n in re.findall(r'byte_ladder_cells_grouped: (\d+)', candidate)]
assert any(n >= 4 for n in counts), 'fixture did not reach the byte-ladder transform'
assert not any(int(n) for n in re.findall(r'byte_ladder_cells_grouped: (\d+)', disabled))
print('Accepted byte-ladder cells:', counts)
PY
printf 'All byte-ladder arms agree; candidate reached the transform.\n'
