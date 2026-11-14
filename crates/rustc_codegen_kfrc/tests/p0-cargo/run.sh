#!/usr/bin/env bash
# Per-package codegen backend selection: kfrc library, LLVM Cargo driver.
set -euo pipefail
backend=$(realpath "${1:?backend dylib}")
work=${2:?result directory}
mkdir -p "$work"
work=$(realpath "$work")
export RUSTC_BOOTSTRAP=1
toolchain=${KFRC_TOOLCHAIN:-1.94.1}
for arm in kfrc llvm; do
    project="$work/$arm"
    mkdir -p "$project/logic/src" "$project/src"
    cat > "$project/Cargo.toml" <<'TOML'
cargo-features = ["codegen-backend"]
[package]
name = "poc-cargo-driver"
version = "0.0.0"
edition = "2021"
[dependencies]
poc-cargo-logic = { path = "logic" }
[workspace]
[profile.dev]
panic = "abort"
overflow-checks = false
debug-assertions = false
[profile.dev.package.poc-cargo-logic]
TOML
    if [ "$arm" = kfrc ]; then
        printf 'codegen-backend = "%s"\n' "$backend" >> "$project/Cargo.toml"
    fi
    cat > "$project/logic/Cargo.toml" <<'TOML'
[package]
name = "poc-cargo-logic"
version = "0.0.0"
edition = "2021"
TOML
    cat > "$project/logic/src/lib.rs" <<'RS'
#[inline(never)]
pub fn workload(mut x: u64, n: u32) -> u64 {
    let mut i = 0;
    while i < n {
        x = x.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        if x & 7 == 0 { x ^= x >> 23; }
        i += 1;
    }
    x
}
RS
    cat > "$project/src/main.rs" <<'RS'
fn main() {
    for seed in [0, 1, 7, 42, u64::MAX] {
        println!("{}", poc_cargo_logic::workload(seed, 400));
    }
}
RS
    (cd "$project" && cargo +"$toolchain" build -vv --offline) > "$project/build.log" 2>&1
    "$project/target/debug/poc-cargo-driver" > "$work/$arm.out"
done
cmp "$work/kfrc.out" "$work/llvm.out"
# Cargo's rustc invocation is retained in build.log; provenance additionally
# checks the library's relocatable object was written by kfrc.
objects="$work/kfrc/objects"
mkdir -p "$objects"
rlib=$(find "$work/kfrc/target/debug/deps" -maxdepth 1 -name 'libpoc_cargo_logic-*.rlib' -print -quit)
(cd "$objects" && ar x "$rlib")
readelf -s "$objects"/*.o | grep -q 'FILE.*kfrc'
printf 'PASS: Cargo selected kfrc for the library; 5 output lines identical to LLVM\n'
