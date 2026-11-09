#!/usr/bin/env bash
# Compile unchanged P1 integer modules independently of unrelated P1 fixtures.
set -euo pipefail
backend=$(realpath "${1:?backend dylib}")
work=${2:?fresh result directory}
here=$(cd "$(dirname "$0")/../../crates/rustc_codegen_kfrc/tests/p1-memory" && pwd)
mkdir "$work"
work=$(realpath "$work")
export RUSTC_BOOTSTRAP=1
python3 - "$here" "$work" <<'PY'
import hashlib, json, pathlib, sys
source, work = map(pathlib.Path, sys.argv[1:])
driver = (source / "driver.rs").read_text()
start = driver.index("fn bit_operation_probes()")
end = driver.index("\nfn ", driver.index("fn saturation_probes()") + 1)
(work / "driver.rs").write_text("extern crate memory;\n" + driver[start:end] +
    "\nfn main() { bit_operation_probes(); saturation_probes(); }\n")
text = "#![feature(core_intrinsics, funnel_shifts)]\n#![allow(internal_features)]\n"
for module in ("bitops", "saturation"):
    text += f"#[path = {json.dumps(str(source / (module + '.rs')))}] pub mod {module};\n"
(work / "memory.rs").write_text(text)
inputs = [source / name for name in ("bitops.rs", "saturation.rs", "driver.rs")]
inputs += [work / "memory.rs", work / "driver.rs"]
(work / "source.json").write_text(json.dumps({str(p): hashlib.sha256(p.read_bytes()).hexdigest()
                                            for p in inputs}, indent=2) + "\n")
PY
flags=(--edition=2021 -Cpanic=abort -Copt-level=0 -Coverflow-checks=on -Cdebug-assertions=off -Zmir-opt-level=0)
for arm in llvm kfrc; do
    mkdir "$work/$arm"
    backend_flags=()
    if [ "$arm" = kfrc ]; then backend_flags=(-Zcodegen-backend="$backend"); fi
    (
        set -x
        rustc +1.94.1 "${flags[@]}" "${backend_flags[@]}" --crate-name=memory --crate-type=rlib "$work/memory.rs" --out-dir="$work/$arm"
        rustc +1.94.1 "${flags[@]}" "$work/driver.rs" --extern="memory=$work/$arm/libmemory.rlib" -o "$work/$arm/driver"
    ) > "$work/$arm/build.log" 2>&1
    "$work/$arm/driver" > "$work/$arm/output.txt" 2> "$work/$arm/stderr.txt"
done
cmp "$work/llvm/output.txt" "$work/kfrc/output.txt"
mkdir "$work/kfrc/objects"
(cd "$work/kfrc/objects" && ar x ../libmemory.rlib)
readelf -Ws "$work/kfrc/objects/"*.o > "$work/kfrc/object-symbols.txt"
grep -q 'FILE.*kfrc' "$work/kfrc/object-symbols.txt"
sha256sum "$backend" "$work/kfrc/objects/"*.o "$work/kfrc/driver" "$work/llvm/driver" > "$work/output.sha256"
printf 'PASS: 500 rotate/funnel boundary outputs and 131872 saturation operand pairs, LLVM differential and mathematical clamp reference\n'
