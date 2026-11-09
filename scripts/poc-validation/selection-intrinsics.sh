#!/usr/bin/env bash
# Independently execute the unchanged P1 selection module and driver assertions.
set -euo pipefail
backend=$(realpath "${1:?backend dylib}")
work=${2:?fresh result directory}
here=$(cd "$(dirname "$0")/../../crates/rustc_codegen_kfrc/tests/p1-memory" && pwd)
mkdir "$work"
work=$(realpath "$work")
export RUSTC_BOOTSTRAP=1
python3 - "$here" "$work" "$0" <<'PY'
import hashlib, json, pathlib, sys
source, work, script = map(pathlib.Path, sys.argv[1:])
driver = (source / "driver.rs").read_text()
start = driver.index("fn selection_probes()")
end = driver.index("\nfn ", start + 1)
(work / "driver.rs").write_text("extern crate memory;\n" + driver[start:end] +
                               "\nfn main() { selection_probes(); }\n")
(work / "memory.rs").write_text(
    "#![feature(core_intrinsics)]\n#![allow(internal_features)]\n" +
    f"#[path = {json.dumps(str(source / 'select.rs'))}] pub mod select;\n")
inputs = [source / "select.rs", source / "driver.rs", work / "memory.rs", work / "driver.rs", script]
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
        rustc +"${KFRC_TOOLCHAIN:-1.94.1}" "${flags[@]}" "${backend_flags[@]}" --crate-name=memory --crate-type=rlib "$work/memory.rs" --out-dir="$work/$arm"
        rustc +"${KFRC_TOOLCHAIN:-1.94.1}" "${flags[@]}" "$work/driver.rs" --extern="memory=$work/$arm/libmemory.rlib" -o "$work/$arm/driver"
    ) > "$work/$arm/build.log" 2>&1
    "$work/$arm/driver" > "$work/$arm/output.txt" 2> "$work/$arm/stderr.txt"
done
cmp "$work/llvm/output.txt" "$work/kfrc/output.txt"
test "$(wc -l < "$work/kfrc/output.txt")" -eq 54
mkdir "$work/kfrc/objects"
(cd "$work/kfrc/objects" && ar x ../libmemory.rlib)
readelf -Ws "$work/kfrc/objects/"*.o > "$work/kfrc/object-symbols.txt"
grep -q 'FILE.*kfrc' "$work/kfrc/object-symbols.txt"
sha256sum "$backend" "$work/kfrc/objects/"*.o "$work/kfrc/driver" "$work/llvm/driver" > "$work/output.sha256"
printf 'PASS: 54 selection outputs, unchanged P1 assertions and LLVM differential; full P1 remains separate\n'
