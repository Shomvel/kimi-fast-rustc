#!/usr/bin/env bash
# Later public-API frontier only: no claim about regex-syntax's internal tests.
set -euo pipefail
backend=$(realpath "${1:?backend dylib}")
work=${2:?fresh result directory}
here=$(cd "$(dirname "$0")" && pwd)
mkdir -p "$work"
work=$(realpath "$work")
export RUSTC_BOOTSTRAP=1
toolchain=${KFRC_TOOLCHAIN:-1.94.1}
source=$(python3 "$here/regex-fetch.py" "$work")
rustc +"$toolchain" -vV > "$work/rustc-version.txt"
sha256sum "$backend" "$here/regex-driver.rs" "$here/regex-fetch.py" "$here/regex-smoke.sh" > "$work/input.sha256"
for arm in llvm kfrc; do
    project="$work/$arm"
    mkdir -p "$project/src"
    python3 - "$project/Cargo.toml" "$source" "$arm" "$backend" <<'PY'
import json, pathlib, sys
manifest, source, arm, backend = sys.argv[1:]
text = '''cargo-features = ["codegen-backend"]
[package]
name = "poc-regex-driver"
version = "0.0.0"
edition = "2021"
[workspace]
[dependencies]
regex-syntax = { path = SOURCE, default-features = true }
[profile.dev]
panic = "abort"
overflow-checks = false
debug-assertions = false
opt-level = 0
debug = 0
incremental = false
[profile.dev.package.regex-syntax]
'''.replace("SOURCE", json.dumps(source))
if arm == "kfrc":
    text += "codegen-backend = " + json.dumps(backend) + "\n"
pathlib.Path(manifest).write_text(text)
PY
    cp "$here/regex-driver.rs" "$project/src/main.rs"
    (cd "$project" && CARGO_ENCODED_RUSTFLAGS=$'-Zmir-opt-level=0' cargo +"$toolchain" build -vv) > "$project/build.log" 2>&1
    "$project/target/debug/poc-regex-driver" > "$project/workload.out"
    rlib=$(find "$project/target/debug/deps" -maxdepth 1 -name 'libregex_syntax-*.rlib' -print -quit)
    test -n "$rlib"
    mkdir -p "$project/objects"
    (cd "$project/objects" && ar x "$rlib")
    readelf -Ws "$project"/objects/*.o > "$project/object-symbols.txt"
    if [ "$arm" = kfrc ]; then grep -q 'FILE.*kfrc' "$project/object-symbols.txt"; fi
    sha256sum "$rlib" "$project/target/debug/poc-regex-driver" > "$project/outputs.sha256"
done
cmp "$work/kfrc/workload.out" "$work/llvm/workload.out"
printf 'PASS: regex-syntax 0.8.8 public-API corpus outputs identical; internal tests not run\n'
