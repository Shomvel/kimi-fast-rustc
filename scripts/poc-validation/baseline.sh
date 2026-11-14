#!/usr/bin/env bash
set -euo pipefail
repo=${1:?source directory}
out=${2:?result directory}
mkdir -p "$out"
cd "$repo"
export RUSTC_BOOTSTRAP=1 CARGO_BUILD_JOBS=4
rustc +1.94.1 -Vv > "$out/toolchain.txt"
cargo +1.94.1 -V >> "$out/toolchain.txt"
cargo +1.94.1 test --workspace -- --test-threads=1 > "$out/workspace-tests.log" 2>&1
(cd crates/rustc_codegen_kfrc && cargo +1.94.1 build) > "$out/backend-build.log" 2>&1
backend="$repo/crates/rustc_codegen_kfrc/target/debug/librustc_codegen_kfrc.so"
sha256sum "$backend" > "$out/backend.sha256"
sh crates/rustc_codegen_kfrc/tests/p0-scalar/run.sh "$backend" "$out/scalar" > "$out/scalar.log" 2>&1
sh crates/rustc_codegen_kfrc/tests/p0-scalar/generated/run.sh "$backend" "$out/generated" 1 5 40 > "$out/generated.log" 2>&1
cat "$out/scalar.log" "$out/generated.log"
printf 'BASELINE PASS\n'
