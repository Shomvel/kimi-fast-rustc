#!/usr/bin/env bash
# 统计 sysroot 中 #[rustc_*] 属性（含 #![rustc_*] 与 cfg_attr 里的 rustc_*）按种类、按 crate 的数量，
# 以及 staged_api 相关属性（stable/unstable/rustc_const_*、allow_internal_unstable 等）。
# core 的计数包含 #[path] 引入的 core_arch（全部架构目录，另给 x86/x86_64 子计数）与 core_simd。
# 注释行（// 开头）与 tests/benches 文件不计。
#
# 用法：rustc-attrs.sh LIBRARY_DIR [TOP] [COMPILER_DIR]
#   TOP           按总数输出的种类数（默认 80）
#   COMPILER_DIR  rustc 源码的 compiler/ 目录；给出时与 builtin_attrs.rs 的内建属性表对照
set -euo pipefail
lib=${1:?usage: rustc-attrs.sh LIBRARY_DIR [TOP] [COMPILER_DIR]}
top=${2:-80}

files_of() {
  case $1 in
    core) find "$lib/core/src" "$lib/stdarch/crates/core_arch/src" "$lib/portable-simd/crates/core_simd/src" -name '*.rs' ;;
    core_arch_x86) find "$lib/stdarch/crates/core_arch/src/x86" "$lib/stdarch/crates/core_arch/src/x86_64" -name '*.rs' ;;
    *) find "$lib/$1" -name '*.rs' ;;
  esac | grep -v -E '/tests?(/|\.rs$)|/benches/|/examples/'
}

attrs_of() {
  files_of "$1" | xargs awk '
    /^[[:space:]]*\/\// { next }
    {
      t = $0
      while (match(t, /#!?\[(cfg_attr\([^]]*, *)?rustc_[a-z_0-9]+/)) {
        a = substr(t, RSTART, RLENGTH); sub(/.*rustc_/, "rustc_", a); print a
        t = substr(t, RSTART + RLENGTH)
      }
    }'
}

crates="core alloc std panic_unwind panic_abort unwind std_detect compiler-builtins/compiler-builtins"
tmp=$(mktemp -d)
trap 'rm -r "$tmp"' EXIT
for c in $crates core_arch_x86; do
  attrs_of "$c" | sort | uniq -c | awk -v c="${c%%/*}" '{print c, $2, $1}'
done > "$tmp/all"

echo "# 每个 crate 的 rustc_* 属性总数与种类数：crate uses kinds"
awk '{u[$1] += $3; k[$1]++} END {for (c in u) print c, u[c], k[c]}' "$tmp/all" | sort -k2,2nr
echo "# 不同种类（core+alloc+std+panic_*+unwind+std_detect+compiler_builtins，不含 core_arch_x86 重复计数）: $(awk '$1 != "core_arch_x86" {print $2}' "$tmp/all" | sort -u | wc -l | tr -d ' ')"
echo "# 按种类：attr total core alloc std other core_arch_x86"
awk '$1 != "core_arch_x86" {t[$2] += $3}
     {v[$2, $1] = $3}
     END {
       for (a in t) {
         o = t[a] - v[a, "core"] - v[a, "alloc"] - v[a, "std"]
         printf "%s %d %d %d %d %d %d\n", a, t[a], v[a, "core"], v[a, "alloc"], v[a, "std"], o, v[a, "core_arch_x86"]
       }
     }' "$tmp/all" | sort -k2,2nr | head -n "$top"

echo "# staged_api 与内部可见性相关：attr core alloc std"
for c in core alloc std; do
  files_of "$c" | xargs grep -hoE '^[^/]*#\[(stable|unstable|rustc_const_stable|rustc_const_unstable|rustc_const_stable_indirect|rustc_allow_const_fn_unstable|allow_internal_unstable|allow_internal_unsafe|rustc_deprecated_safe_2024|deprecated|doc\(hidden\))\b' |
    sed -E 's/^[^#]*#\[//; s/\($//' | sort | uniq -c | awk -v c="$c" '{print c, $2, $1}'
done | awk '{v[$2, $1] = $3; a[$2] = 1} END {for (x in a) print x, v[x, "core"] + 0, v[x, "alloc"] + 0, v[x, "std"] + 0}' | sort -k2,2nr

if [ -n "${3:-}" ]; then
  # 编译器认识的 rustc_* 内建属性：rustc_feature/src/builtin_attrs.rs 中以 rustc_ 开头的条目
  ba=$3/rustc_feature/src/builtin_attrs.rs
  grep -oE '^[[:space:]]*((rustc_attr|gated|ungated)!\()?[[:space:]]*(TEST, )?rustc_[a-z_0-9]+,|name: sym::rustc_[a-z_0-9]+,' "$ba" |
    sed -E 's/.*(rustc_[a-z_0-9]+),/\1/' | sort -u > "$tmp/known"
  awk '$1 != "core_arch_x86" {print $2}' "$tmp/all" | sort -u > "$tmp/used"
  echo "# 编译器内建 rustc_* 属性数: $(wc -l < "$tmp/known" | tr -d ' ')；sysroot 用到其中: $(comm -12 "$tmp/known" "$tmp/used" | wc -l | tr -d ' ')"
  echo "# sysroot 用到但不在 builtin_attrs 表（宏生成名或另行解析）: $(comm -13 "$tmp/known" "$tmp/used" | tr '\n' ' ')"
fi
