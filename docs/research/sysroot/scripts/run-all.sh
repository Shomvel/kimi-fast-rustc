#!/usr/bin/env bash
# 依次运行本目录的全部统计脚本，把输出写到 OUT_DIR（每个脚本一个 .txt）。
#
# 用法：run-all.sh LIBRARY_DIR COMPILER_DIR REGISTRY_SRC BENCH_DIR OUT_DIR [RUSTC] [GIT_REPO COMMIT]
#   LIBRARY_DIR   rust-src 的 library/（本次：Homebrew rust 1.94.1 附带的 rust-src）
#   COMPILER_DIR  rustc 1.94.1 源码的 compiler/
#   REGISTRY_SRC  ~/.cargo/registry/src/<index>（已 cargo fetch --locked）
#   BENCH_DIR     含 ripgrep/ 与 cargo/ 两个检出的目录
#   OUT_DIR       输出目录
#   RUSTC         可选，打印 -Vv 核对 rust-src 所属版本
#   GIT_REPO COMMIT  可选，核对派生树 library/ 是否等于官方 commit
set -euo pipefail
lib=${1:?usage: run-all.sh LIBRARY_DIR COMPILER_DIR REGISTRY_SRC BENCH_DIR OUT_DIR [RUSTC] [GIT_REPO COMMIT]}
comp=${2:?}; reg=${3:?}; bench=${4:?}; out=${5:?}
rustc=${6:--}; repo=${7:--}; commit=${8:--}
here=$(cd "$(dirname "$0")" && pwd)
mkdir -p "$out"
t=x86_64-unknown-linux-gnu

other=-
[ "$repo" != - ] && other=$repo/library
"$here/sources-check.sh" "$lib" "$rustc" "$repo" "$commit" "$other" > "$out/sources.txt"
"$here/intrinsics.sh" "$lib" > "$out/intrinsics.txt"
"$here/intrinsics.sh" "$lib" list > "$out/intrinsics-list.txt"
"$here/lang-items.sh" "$lib" "$comp" > "$out/lang-items.txt"
"$here/rustc-attrs.sh" "$lib" 100 "$comp" > "$out/rustc-attrs.txt"
"$here/features.sh" "$lib" "$comp" list > "$out/features.txt"
"$here/builtin-macros.sh" "$lib" "$comp" > "$out/builtin-macros.txt"
"$here/llvm-asm-arch.sh" "$lib" > "$out/llvm-asm-arch.txt"
"$here/llvm-asm-arch.sh" "$lib" list > "$out/x86-fns.txt"
"$here/runtime.sh" "$lib" > "$out/runtime.txt"

# 依赖闭包：std（官方构建默认 features：panic-unwind + backtrace）、ripgrep 默认、Cargo 默认
RUSTC_BOOTSTRAP=1 "$here/deps-closure.sh" "$lib/Cargo.toml" "$reg" "$t" sysroot backtrace,panic-unwind > "$out/closure-sysroot.txt"
"$here/deps-closure.sh" "$bench/ripgrep/Cargo.toml" "$reg" "$t" > "$out/closure-ripgrep.txt"
"$here/deps-closure.sh" "$bench/cargo/Cargo.toml" "$reg" "$t" cargo > "$out/closure-cargo.txt"
for p in sysroot ripgrep cargo; do
  "$here/deps-scan.sh" "$out/closure-$p.txt" "$out/x86-fns.txt" > "$out/scan-$p.txt"
  "$here/deps-scan.sh" "$out/closure-$p.txt" "$out/x86-fns.txt" x86 | sed -n '/每个 crate 引用/,$p' > "$out/scan-$p-x86.txt"
  "$here/asm-sites.sh" "$out/closure-$p.txt" > "$out/asm-$p.txt"
done
echo "done: $out"
