#!/usr/bin/env bash
# 统计 core::intrinsics 中 #[rustc_intrinsic] 的定义，按类别分组，并统计在
# core/alloc/std（含 #[path] 引入的 core_arch x86/x86_64 与 core_simd）中被引用的数量。
#
# 引用判定（近似，偏宽）：在非定义文件中出现 `intrinsics::NAME` / `simd::NAME`，
# 或 NAME 出现在一条含 `intrinsics` 的 use 语句里，或文件 glob 导入 intrinsics::simd::*
# 且出现 `NAME(`。注释行与 tests/benches 文件不计。
#
# 用法：intrinsics.sh LIBRARY_DIR [list]
#   LIBRARY_DIR  rust-src 的 library/ 目录
#   list         额外输出逐项明细（名称 类别 const 有回退体 引用文件数）
set -euo pipefail
lib=${1:?usage: intrinsics.sh LIBRARY_DIR [list]}
mode=${2:-summary}
idir=$lib/core/src/intrinsics

defs=$(for f in "$idir"/mod.rs "$idir"/simd.rs "$idir"/gpu.rs; do
  awk -v file="${f##*/}" '
    /#\[rustc_intrinsic\]/ { pend = 1; next }
    pend && /fn [a-z_0-9]+/ {
      match($0, /fn [a-z_0-9]+/); name = substr($0, RSTART + 3, RLENGTH - 3)
      isconst = ($0 ~ /const (unsafe )?fn/) ? "const" : "-"
      sig = substr($0, RSTART); pend = 0; insig = 1
    }
    insig {
      if (sig == "") sig = $0
      b = index(sig, "{"); s = index(sig, ";")
      if (b || s) {
        body = (b && (!s || b < s)) ? "body" : "-"
        print name, file, isconst, body; insig = 0; sig = ""
      } else sig = ""
    }' "$f"
done)

# 引用扫描范围：core（除 intrinsics 定义文件）、alloc、std、core_arch 的 x86 部分、core_simd
scan_files() {
  find "$lib/core/src" "$lib/alloc/src" "$lib/std/src" \
       "$lib/stdarch/crates/core_arch/src/x86" "$lib/stdarch/crates/core_arch/src/x86_64" \
       "$lib/portable-simd/crates/core_simd/src" -name '*.rs' 2>/dev/null |
    grep -v -E '/core/src/intrinsics/(mod|simd|gpu|fallback|bounds)\.rs$|/tests?(/|\.rs$)|/benches/'
  find "$lib/stdarch/crates/core_arch/src" -maxdepth 1 -name '*.rs'
}

tmp=$(mktemp -d)
trap 'rm -r "$tmp"' EXIT
echo "$defs" | awk '{print $1}' | sort -u > "$tmp/names"
scan_files | while read -r f; do
  awk -v names="$tmp/names" '
    BEGIN { while ((getline nm < names) > 0) known[nm] = 1 }
    /^[[:space:]]*\/\// { next }
    {
      line = $0
      # use 语句可能跨行：累积到分号后整体判断是否涉及 intrinsics
      if (inuse || line ~ /^[[:space:]]*(pub(\([a-z]+\))? )?use /) {
        stmt = stmt " " line; inuse = (line !~ /;/)
        if (inuse) next
        if (stmt ~ /intrinsics/) {
          if (stmt ~ /intrinsics::simd::\*/) simdglob = 1
          t = stmt; gsub(/[^a-z_0-9]+/, " ", t); k = split(t, w, " ")
          for (i = 1; i <= k; i++) if (w[i] in known) hit[w[i]] = 1
        }
        stmt = ""; next
      }
      t = line
      while (match(t, /(intrinsics|simd)(::[a-z_0-9]+)+/)) {
        w0 = substr(t, RSTART, RLENGTH); k = split(w0, w, "::")
        for (i = 2; i <= k; i++) if (w[i] in known) hit[w[i]] = 1
        t = substr(t, RSTART + RLENGTH)
      }
      if (simdglob) {
        t = line
        while (match(t, /simd_[a-z_0-9]+[[:space:]]*(::<[^(]*>)?\(/)) {
          w0 = substr(t, RSTART, RLENGTH); sub(/[^a-z_0-9].*/, "", w0)
          if (w0 in known) hit[w0] = 1
          t = substr(t, RSTART + RLENGTH)
        }
      }
    }
    END { for (h in hit) print h }' "$f"
done | sort | uniq -c | awk '{print $2, $1}' > "$tmp/refs"

table=$(echo "$defs" | awk -v refs="$tmp/refs" '
  function cat(n) {
    if (n ~ /^atomic_/) return "atomic"
    if (n ~ /^simd_/) return "simd-generic"
    if (n ~ /^(const_allocate|const_deallocate|const_make_global|const_eval_select|ptr_guaranteed_cmp|is_val_statically_known|type_of|type_id_eq)$/) return "const-eval"
    if (n ~ /^(ub_checks|overflow_checks|contract_check_requires|contract_check_ensures)$/) return "check-config"
    if (n ~ /^(needs_drop|size_of|align_of|offset_of|variant_count|size_of_val|align_of_val|type_name|type_id|vtable_size|vtable_align|vtable_for|assert_inhabited|assert_zero_valid|assert_mem_uninitialized_valid|caller_location|discriminant_value)$/) return "type-layout"
    if (n ~ /f(16|32|64|128)$/ || n ~ /_fast$|_algebraic$/ || n == "float_to_int_unchecked") return "float"
    if (n ~ /^(ctpop|ctlz|ctlz_nonzero|cttz|cttz_nonzero|bswap|bitreverse|three_way_compare|disjoint_bitor|add_with_overflow|sub_with_overflow|mul_with_overflow|carrying_mul_add|exact_div|rotate_left|rotate_right)$/ || n ~ /^(unchecked|wrapping|saturating)_/) return "int"
    if (n ~ /^(copy|copy_nonoverlapping|write_bytes|read_via_copy|write_via_move|offset|arith_offset|slice_get_unchecked|ptr_mask|ptr_offset_from|ptr_offset_from_unsigned|raw_eq|compare_bytes|typed_swap_nonoverlapping|aggregate_raw_ptr|ptr_metadata|transmute|transmute_unchecked|nontemporal_store)$/ || n ~ /volatile/ || n ~ /^prefetch_/) return "memory-ptr"
    if (n ~ /^(abort|unreachable|assume|cold_path|select_unpredictable|breakpoint|black_box|forget|catch_unwind)$/) return "control-hint"
    if (n ~ /^va_/) return "varargs"
    return "other"
  }
  BEGIN { while ((getline l < refs) > 0) { split(l, p, " "); rc[p[1]] = p[2] } }
  { c = cat($1); print $1, c, $3, $4, ($1 in rc ? rc[$1] : 0) }')

if [ "$mode" = list ]; then
  echo "# name category const fallback ref_files"
  echo "$table" | sort -k2,2 -k1,1
  exit 0
fi

echo "# 定义：按文件"
echo "$defs" | awk '{c[$2]++} END {for (k in c) print k, c[k]}' | sort
echo "# 按类别：category defined const_fn with_fallback_body referenced"
echo "$table" | awk '{d[$2]++; if ($3 == "const") k[$2]++; if ($4 == "body") b[$2]++; if ($5 > 0) u[$2]++}
  END {for (c in d) printf "%s %d %d %d %d\n", c, d[c], k[c], b[c], u[c]}' | sort -k2,2nr
echo "$table" | awk '{d++; if ($3 == "const") k++; if ($4 == "body") b++; if ($5 > 0) u++}
  END {printf "TOTAL %d %d %d %d\n", d, k, b, u}'
echo "# float 类中 f16/f128 版本（名称以 f16/f128 结尾）：defined referenced"
echo "$table" | awk '$2 == "float" && $1 ~ /f(16|128)$/ {d++; if ($5 > 0) u++} END {print d + 0, u + 0}'
echo "# 未被引用的定义（$(echo "$table" | awk '$5 == 0' | wc -l | tr -d ' ') 个）"
echo "$table" | awk '$5 == 0 {print $1}' | sort | tr '\n' ' '; echo
