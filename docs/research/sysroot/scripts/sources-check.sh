#!/usr/bin/env bash
# 核对 std 源码来源并给出 sysroot 各 crate 的规模。
#   1. 若给出 RUSTC，打印 rustc -Vv（确认 rust-src 所属工具链的版本与 commit）。
#   2. 若给出 GIT_REPO 与 COMMIT：打印该 commit 与 HEAD 的 library/ tree 哈希、HEAD 相对 COMMIT 的
#      library/ 改动文件数、工作区未提交/未跟踪文件数（确认派生树的 library/ 未被本地修改）。
#   3. 若给出 OTHER_LIBRARY_DIR：diff -rq 两份 library/，按“仅一侧存在/内容不同”分类计数。
#   4. 各 crate 的 .rs 文件数与非注释行数（排除 tests/benches/examples）。
#
# 用法：sources-check.sh LIBRARY_DIR [RUSTC] [GIT_REPO COMMIT] [OTHER_LIBRARY_DIR]
#   不需要的位置参数传 -
set -euo pipefail
lib=${1:?usage: sources-check.sh LIBRARY_DIR [RUSTC] [GIT_REPO COMMIT] [OTHER_LIBRARY_DIR]}
rustc=${2:--}
repo=${3:--}
commit=${4:--}
other=${5:--}

if [ "$rustc" != - ]; then
  echo "# rustc -Vv"
  "$rustc" -Vv
fi

if [ "$repo" != - ]; then
  echo "# git: $commit 与 HEAD 的 library/ tree"
  git -C "$repo" rev-parse "$commit:library" HEAD:library
  echo "HEAD 相对 $commit 的 library/ 改动文件数: $(git -C "$repo" diff --name-only "$commit" HEAD -- library | wc -l | tr -d ' ')"
  echo "library/ 未提交改动: $(git -C "$repo" status --short -- library | wc -l | tr -d ' ')"
  echo "library/ 未跟踪文件: $(git -C "$repo" ls-files --others --exclude-standard -- library | wc -l | tr -d ' ')"
  echo "library/ 下的子模块（160000 条目）: $(git -C "$repo" ls-tree -r "$commit" -- library | awk '$1 == "160000" {print $4}' | tr '\n' ' ')"
fi

if [ "$other" != - ]; then
  echo "# diff -rq LIBRARY_DIR OTHER_LIBRARY_DIR（顶层第一段路径汇总）"
  diff -rq "$lib" "$other" 2>/dev/null | awk -v a="$lib" -v b="$other" '
    /^Only in / {
      p = $3; sub(/:$/, "", p); side = (index(p, a) == 1) ? "only-first" : "only-second"
      base = (side == "only-first") ? a : b
      rel = substr(p "/" $4, length(base) + 2); split(rel, s, "/")
      c[side " " s[1]]++; next
    }
    /^Files / { rel = substr($2, length(a) + 2); split(rel, s, "/"); c["differ " s[1]]++ }
    END { for (k in c) print k, c[k] }' | sort || true
fi

echo "# 规模：crate rs_files code_lines（非空、非 // 注释行）"
size() {
  local label=$1; shift
  local files
  files=$(find "$@" -name '*.rs' 2>/dev/null | grep -v -E '/(tests|benches|examples)/|/tests\.rs$' || true)
  local n l
  n=$(echo "$files" | grep -c . || true)
  # shellcheck disable=SC2086
  l=$(echo "$files" | xargs cat 2>/dev/null | grep -v -E '^[[:space:]]*(//|$)' | wc -l | tr -d ' ')
  echo "$label $n $l"
}
size core "$lib/core/src"
size core_arch-x86 "$lib/stdarch/crates/core_arch/src/x86" "$lib/stdarch/crates/core_arch/src/x86_64"
size core_arch-all-arch "$lib/stdarch/crates/core_arch/src"
size core_simd "$lib/portable-simd/crates/core_simd/src"
size alloc "$lib/alloc/src"
size std "$lib/std/src"
size std-sys-pal-unix "$lib/std/src/sys/pal/unix"
size backtrace "$lib/backtrace/src"
size std_detect "$lib/std_detect/src"
size panic_unwind "$lib/panic_unwind/src"
size panic_abort "$lib/panic_abort/src"
size unwind "$lib/unwind/src"
size compiler_builtins "$lib/compiler-builtins/compiler-builtins/src"
size compiler_builtins-libm "$lib/compiler-builtins/libm/src"
size proc_macro "$lib/proc_macro/src"
size test "$lib/test/src"
echo "注：core_arch-all-arch 一行是 core_arch/src 全部（含 x86），core 通过 #[path] 引入全部架构但按 cfg 只编译目标架构。"
