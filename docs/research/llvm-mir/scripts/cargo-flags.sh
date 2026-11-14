#!/usr/bin/env bash
# 记录 cargo 实际传给 rustc 的与流水线相关的参数：--emit、opt-level、embed-bitcode、
# codegen-units、lto，以及 --extern 指向 .rmeta 还是 .rlib（rmeta 流水线）。
# 工作区：lib 链 a <- b，bin app 依赖 b。
# 用法：cargo-flags.sh WORKDIR [CARGO]
set -euo pipefail
work=${1:?usage: cargo-flags.sh WORKDIR [CARGO]}
cargo=${2:-cargo}
ws=$work/cargo-ws
mkdir -p "$ws/a/src" "$ws/b/src" "$ws/app/src"
cat > "$ws/Cargo.toml" <<'T'
[workspace]
members = ["a", "b", "app"]
resolver = "2"
T
cat > "$ws/a/Cargo.toml" <<'T'
[package]
name = "a"
version = "0.1.0"
edition = "2021"
T
cat > "$ws/b/Cargo.toml" <<'T'
[package]
name = "b"
version = "0.1.0"
edition = "2021"
[dependencies]
a = { path = "../a" }
T
cat > "$ws/app/Cargo.toml" <<'T'
[package]
name = "app"
version = "0.1.0"
edition = "2021"
[dependencies]
b = { path = "../b" }
T
echo 'pub fn f(x: u32) -> u32 { x.wrapping_mul(3) }' > "$ws/a/src/lib.rs"
echo 'pub fn g(x: u32) -> u32 { a::f(x) + 1 }' > "$ws/b/src/lib.rs"
echo 'fn main() { println!("{}", b::g(std::env::args().count() as u32)); }' > "$ws/app/src/main.rs"
"$cargo" --version > "$work/cargo-version.txt"

summarize() { # LOG -> 每个 rustc 调用一行
  grep -E '^ *Running `' "$1" | grep -- '--crate-name' | while IFS= read -r l; do
    name=$(printf '%s\n' "$l" | sed -E 's/.*--crate-name ([^ ]+).*/\1/')
    emit=$(printf '%s\n' "$l" | grep -oE -- '--emit=[^ ]+' || true)
    flags=$(printf '%s\n' "$l" | grep -oE -- '-C (opt-level|embed-bitcode|codegen-units|lto|linker-plugin-lto)[^ ]*' | paste -sd' ' - || true)
    ext=$(printf '%s\n' "$l" | grep -oE -- '--extern [^ ]+' | sed -E 's#--extern ([^=]+)=.*/lib[^.]+\.(rmeta|rlib).*#\1:\2#' | paste -sd' ' - || true)
    printf '%s\t%s\t%s\t%s\n' "$name" "$emit" "$flags" "$ext"
  done
}

for profile in release dev; do
  for lto in default thin fat off; do
    tgt=$work/target-$profile-$lto
    rm -rf "$tgt"
    args=(build -v --offline --manifest-path "$ws/Cargo.toml")
    [ "$profile" = release ] && args+=(--release)
    if [ "$lto" = default ]; then
      CARGO_TARGET_DIR=$tgt "$cargo" "${args[@]}" > "$work/log-$profile-$lto.txt" 2>&1
    else
      up=$(printf '%s' "$profile" | tr a-z A-Z)
      env CARGO_TARGET_DIR="$tgt" "CARGO_PROFILE_${up}_LTO=$lto" "$cargo" "${args[@]}" \
        > "$work/log-$profile-$lto.txt" 2>&1
    fi
    printf '## profile=%s lto=%s\ncrate\temit\tcodegen flags\textern\n' "$profile" "$lto"
    summarize "$work/log-$profile-$lto.txt"
    echo
  done
done > "$work/cargo-flags.tsv"
cat "$work/cargo-flags.tsv"
