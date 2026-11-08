#!/usr/bin/env bash
# 统计 sysroot 各 crate 中的 #[lang = "..."] 定义，并与编译器 language_item_table! 对照：
# 编译器认识多少、库里定义多少、按目标种类和语义类别分组、哪些编译器 lang item 库里没有定义。
# 另统计“弱 lang item”式的入口属性（#[panic_handler]、#[global_allocator] 等）。
#
# 用法：lang-items.sh LIBRARY_DIR COMPILER_DIR
#   LIBRARY_DIR   rust-src 的 library/ 目录
#   COMPILER_DIR  rustc 源码的 compiler/ 目录（读 rustc_hir/src/lang_items.rs）
set -euo pipefail
lib=${1:?usage: lang-items.sh LIBRARY_DIR COMPILER_DIR}
comp=${2:?usage: lang-items.sh LIBRARY_DIR COMPILER_DIR}
tmp=$(mktemp -d)
trap 'rm -r "$tmp"' EXIT

crates="core alloc std panic_unwind panic_abort unwind compiler-builtins/compiler-builtins"
for c in $crates; do
  find "$lib/$c" -name '*.rs' | grep -v -E '/tests?(/|\.rs$)|/benches/' |
    { xargs grep -hoE 'lang *= *"[A-Za-z_0-9]+"' || true; } |
    sed -E 's/.*"([A-Za-z_0-9]+)"/\1/' | awk -v c="${c%%/*}" '{print c, $1}'
done > "$tmp/lib0"
# core 用宏 `#[lang = stringify!($lang)]` 生成 panic_const_* 系列
awk '/panic_const! \{/ {on = 1; next} on && /^[[:space:]]*\}/ {on = 0} on && /^[[:space:]]+panic_const_[a-z_]+ =/ {sub(/^[[:space:]]+/, ""); sub(/ =.*/, ""); print "core", $0}' \
  "$lib/core/src/panicking.rs" >> "$tmp/lib0"
sort -u "$tmp/lib0" > "$tmp/lib"

# 编译器表：Variant, sym::name, getter, Target::Kind, requirement
awk '/^language_item_table! \{/,/^\}/' "$comp/rustc_hir/src/lang_items.rs" |
  grep -E '^[[:space:]]+[A-Z][A-Za-z0-9]*,[[:space:]]+(sym|kw)::' |
  sed -E 's/^[[:space:]]+([A-Za-z0-9]+),[[:space:]]+(sym|kw)::([A-Za-z_0-9]+),[[:space:]]+[a-z_0-9]+,[[:space:]]+Target::([A-Za-z]+).*/\3 \4/' |
  sed -E 's/^Fn /fn /' \
  > "$tmp/comp"

cat_of() {
  awk '
  function cat(n) {
    if (n ~ /^(panic|begin_panic|const_panic|eh_|panic_)/ || n ~ /panic/) return "panic-unwind"
    if (n ~ /^(add|sub|mul|div|rem|neg|not|bitxor|bitand|bitor|shl|shr)(_assign)?$/) return "operator"
    if (n ~ /^(index|index_mut|deref|deref_mut|deref_target|deref_pure|receiver|receiver_target|legacy_receiver|eq|partial_ord|cmp_ord_max|cmp_ord_min|cmp_partialord_cmp|cmp_partialord_lt|cmp_partialord_le|cmp_partialord_gt|cmp_partialord_ge|not_trait)$/) return "operator"
    if (n ~ /^(fn|fn_mut|fn_once|fn_once_output|async_fn|async_fn_mut|async_fn_once|call_ref_future|call_once_future|async_fn_kind_helper|async_fn_kind_upvars|async_fn_once_output|fn_ptr_trait|fn_ptr_addr|tuple_trait)$/) return "closure-fn"
    if (n ~ /coroutine|future|async|poll|context|gen_|iterator|into_iter|next|^iter/ || n ~ /^(get_context|ResumeTy|pin|unpin|new_unchecked|Poll|Ready|Pending|Context)$|^AsyncGen/) return "iter-async-coroutine"
    if (n ~ /^(sized|meta_sized|pointee_sized|unsize|copy|clone|clone_fn|use_cloned|trivial_clone|sync|freeze|unsafe_unpin|structural_peq|discriminant_kind|discriminant_type|pointee_trait|metadata_type|dyn_metadata|destruct|transmute_trait|transmute_opts|bikeshed_guaranteed_no_drop|phantom_data|manually_drop|unsafe_cell|unsafe_cell_raw_get|unsafe_pinned|maybe_uninit|coerce_unsized|dispatch_from_dyn|coerce_pointee_validated|const_param_ty|unsized_const_param_ty|default_trait[0-9]*|drop|field_base|field_type|field_offset|aligned)$/ || n ~ /^(mem_|global_alloc)/) return "type-marker"
    if (n ~ /alloc|box|exchange_malloc|drop_in_place|start$|^start|termination|layout/) return "alloc-drop-entry"
    if (n ~ /^(format_|fmt)/ || n ~ /format/) return "format"
    if (n ~ /^(range|Range)/ || n ~ /range/) return "range"
    if (n ~ /^(option|Option|result|Result|from_|branch|try_|into_|control_flow|from_residual|from_output|from_yeet|String|string|str|slice|const_ptr|mut_ptr|array|c_void|c_str|CStr|va_list)/ || n ~ /^(Some|None|Ok|Err|Continue|Break|Ordering|Try)$/) return "std-type-hook"
    return "other"
  }
  { print $0, cat($NF) }'
}

echo "# 库中定义：crate count（去重后，按 crate）"
awk '{c[$1]++} END {for (k in c) print k, c[k]}' "$tmp/lib" | sort -k2,2nr
awk '{print $2}' "$tmp/lib" | sort -u > "$tmp/libnames"
echo "库中不同 lang item 名称数: $(wc -l < "$tmp/libnames" | tr -d ' ')"
echo "编译器 language_item_table! 条目数: $(wc -l < "$tmp/comp" | tr -d ' ')"
awk '{print $1}' "$tmp/comp" | sort -u > "$tmp/compnames"
echo "两者交集: $(comm -12 "$tmp/libnames" "$tmp/compnames" | wc -l | tr -d ' ')"
echo "# 库中有、编译器表中没有（多为 sym 名与字符串不同）"
comm -23 "$tmp/libnames" "$tmp/compnames" | tr '\n' ' '; echo
echo "# 编译器表中有、库中没有定义"
comm -13 "$tmp/libnames" "$tmp/compnames" | tr '\n' ' '; echo

echo "# 编译器表按 Target 种类：kind total defined_in_library"
awk 'NR == FNR {d[$1] = 1; next} {t[$2]++; if ($1 in d) k[$2]++} END {for (x in t) print x, t[x], k[x] + 0}' \
  "$tmp/libnames" "$tmp/comp" | sort -k2,2nr

echo "# 库中定义按语义类别：category count"
cat_of < "$tmp/libnames" | awk '{c[$2]++} END {for (k in c) print k, c[k]}' | sort -k2,2nr
echo "# 逐项（类别 名称）"
cat_of < "$tmp/libnames" | sort -k2,2 -k1,1 | awk '{a[$2] = a[$2] " " $1} END {for (k in a) print k ":" a[k]}' | sort

echo "# 入口/弱 lang item 类属性：attr count（core alloc std panic_* unwind）"
for c in core alloc std panic_unwind panic_abort unwind; do
  find "$lib/$c" -name '*.rs' | grep -v -E '/tests?(/|\.rs$)|/benches/'
done | xargs grep -hE '^[^/]*#!?\[(panic_handler|global_allocator|alloc_error_handler|rustc_std_internal_symbol|default_lib_allocator|needs_panic_runtime|panic_runtime|needs_allocator|start|no_main|unsafe\(no_mangle\)|no_mangle|compiler_builtins)\]' |
  grep -oE '#\[(panic_handler|global_allocator|alloc_error_handler|rustc_std_internal_symbol|default_lib_allocator|needs_panic_runtime|panic_runtime|needs_allocator|start|no_main|unsafe\(no_mangle\)|no_mangle)\]|#!\[(panic_runtime|needs_panic_runtime|needs_allocator|compiler_builtins|default_lib_allocator)\]' |
  sort | uniq -c | sort -rn
