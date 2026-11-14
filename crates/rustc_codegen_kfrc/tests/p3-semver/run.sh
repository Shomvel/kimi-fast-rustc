#!/usr/bin/env bash
# Actual semver library compiled by the selected backend; unchanged upstream
# integration tests and libtest remain LLVM, with panic=abort in both arms.
set -euo pipefail
backend=$(realpath "${1:?backend dylib}")
work=${2:?fresh result directory}
here=$(cd "$(dirname "$0")" && pwd)
mkdir -p "$work"
work=$(realpath "$work")
export RUSTC_BOOTSTRAP=1
toolchain=${KFRC_TOOLCHAIN:-1.94.1}
source=$(python3 "$here/fetch.py" "$work")
python3 "$here/verify.py" source "$work" > "$work/source-verification-before.json"
sha256sum "$backend" "$here"/*.py "$here/driver.rs" "$here/run.sh" > "$work/input.sha256"
rustc +"$toolchain" -vV > "$work/rustc-version.txt"
cargo +"$toolchain" -V > "$work/cargo-version.txt"
for arm in llvm kfrc; do
    project="$work/$arm"
    mkdir -p "$project/src"
    # JSON string literals are also valid TOML basic strings for these paths.
    python3 - "$project/Cargo.toml" "$source" "$arm" "$backend" <<'PY'
import json
import pathlib
import sys
manifest, source, arm, backend = sys.argv[1:]
text = '''cargo-features = ["codegen-backend"]
[package]
name = "poc-semver-driver"
version = "0.0.0"
edition = "2021"
[workspace]
[dependencies]
semver = { path = SOURCE, default-features = true }
[profile.dev]
panic = "abort"
overflow-checks = false
debug-assertions = false
opt-level = 0
debug = 0
incremental = false
[profile.dev.package.semver]
'''.replace("SOURCE", json.dumps(source))
if arm == "kfrc":
    text += "codegen-backend = " + json.dumps(backend) + "\n"
pathlib.Path(manifest).write_text(text)
PY
    cp "$here/driver.rs" "$project/src/main.rs"
    (cd "$project" && CARGO_ENCODED_RUSTFLAGS=$'-Zmir-opt-level=0' cargo +"$toolchain" build -vv) > "$project/build.log" 2>&1
    "$project/target/debug/poc-semver-driver" > "$project/workload.out"
    rlib=$(find "$project/target/debug/deps" -maxdepth 1 -name 'libsemver-*.rlib' -print -quit)
    test -n "$rlib"
    sha256sum "$rlib" "$project/target/debug/poc-semver-driver" > "$project/outputs.sha256"
    mkdir -p "$project/objects"
    (cd "$project/objects" && ar x "$rlib")
    readelf -Ws "$project"/objects/*.o > "$project/object-symbols.txt"
    readelf -Ws "$project/target/debug/poc-semver-driver" > "$project/driver-symbols.txt"
    if [ "$arm" = kfrc ]; then
        grep -q 'FILE.*kfrc' "$project/object-symbols.txt"
        grep -q -- "$backend" "$project/build.log"
    fi
    python3 "$here/verify.py" objects "$project" "$arm" > "$project/debug-provenance.json"
    for suite in test_autotrait test_identifier test_version test_version_req; do
        rustc +"$toolchain" --edition=2021 --test "$source/tests/$suite.rs" \
            -Cpanic=abort -Zpanic-abort-tests -Copt-level=0 -Coverflow-checks=off \
            -Cdebug-assertions=off -Zmir-opt-level=0 \
            --extern="semver=$rlib" -L "dependency=$project/target/debug/deps" \
            -o "$project/$suite" > "$project/$suite-build.log" 2>&1
        "$project/$suite" --list > "$project/$suite-list.txt"
        "$project/$suite" --test-threads=1 -Zunstable-options --format=json > "$project/$suite.jsonl" 2> "$project/$suite.stderr"
    done
done
python3 "$here/verify.py" source "$work" > "$work/source-verification-after.json"
cmp "$work/kfrc/workload.out" "$work/llvm/workload.out"
python3 "$here/summarize.py" "$work"
