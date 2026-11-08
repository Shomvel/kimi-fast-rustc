#!/usr/bin/env bash
# 统计 sysroot 各 crate 根文件里的 #![feature(...)]（含 cfg_attr 条件打开的），列出对 kfrc 影响大的
# feature 由哪些 crate 打开，并统计 core/alloc/std 源码里对应语言构造的出现次数。
# 注释行与 tests/benches 文件不计；构造计数是文本近似（宏模板内的出现也计入）。
#
# 用法：features.sh LIBRARY_DIR [COMPILER_DIR] [list]
#   COMPILER_DIR  rustc 源码的 compiler/ 目录；给出时按语言/库 feature 分类（读 rustc_feature）
#   list          额外输出每个 crate 的完整 feature 列表
set -euo pipefail
lib=${1:?usage: features.sh LIBRARY_DIR [COMPILER_DIR] [list]}
comp=${2:-}
mode=${3:-summary}
[ "$comp" = list ] && { comp=; mode=list; }
tmp=$(mktemp -d)
trap 'rm -r "$tmp"' EXIT

roots="core=core/src/lib.rs alloc=alloc/src/lib.rs std=std/src/lib.rs
panic_unwind=panic_unwind/src/lib.rs panic_abort=panic_abort/src/lib.rs unwind=unwind/src/lib.rs
std_detect=std_detect/src/lib.rs compiler_builtins=compiler-builtins/compiler-builtins/src/lib.rs
proc_macro=proc_macro/src/lib.rs test=test/src/lib.rs
core_arch(standalone)=stdarch/crates/core_arch/src/lib.rs core_simd(standalone)=portable-simd/crates/core_simd/src/lib.rs"

for r in $roots; do
  name=${r%%=*}; file=$lib/${r#*=}
  awk -v c="$name" '
    /^[[:space:]]*\/\// { next }
    !acc && /^[[:space:]]*#!\[(cfg_attr\(.*)?feature\(/ { acc = 1; buf = "" }
    acc {
      buf = buf " " $0
      if ($0 ~ /\)\]/) {
        cond = (buf ~ /#!\[cfg_attr/) ? "cfg" : "always"
        s = buf; sub(/.*feature\(/, "", s); sub(/\).*/, "", s); gsub(/[[:space:],]+/, " ", s)
        k = split(s, w, " ")
        for (i = 1; i <= k; i++) if (w[i] != "") print c, w[i], cond
        acc = 0
      }
    }' "$file"
done > "$tmp/feat"

echo "# 每个 crate 的 feature 数：crate always cfg"
awk '{n[$1, $3]++; c[$1] = 1} END {for (x in c) print x, n[x, "always"] + 0, n[x, "cfg"] + 0}' "$tmp/feat" | sort -k2,2nr
echo "core+alloc+std 去重后的 feature 数: $(awk '$1 ~ /^(core|alloc|std)$/ {print $2}' "$tmp/feat" | sort -u | wc -l | tr -d ' ')"
echo "全部 sysroot crate（不含 standalone 两项）去重后的 feature 数: $(awk '$1 !~ /standalone/ {print $2}' "$tmp/feat" | sort -u | wc -l | tr -d ' ')"

if [ -n "$comp" ]; then
  # 语言 feature 的来源：rustc_feature 的 declare_features!（unstable/internal/incomplete）；
  # accepted.rs 里的已稳定；其余视为库 feature（只解锁 #[unstable] API，不改变语言）。
  fdir=$comp/rustc_feature/src
  grep -E '^[[:space:]]*\((unstable|internal|incomplete), [a-z_0-9]+,' "$fdir/unstable.rs" |
    sed -E 's/^[[:space:]]*\(([a-z]+), ([a-z_0-9]+),.*/\2 \1/' > "$tmp/langf"
  grep -E '^[[:space:]]*\(accepted, [a-z_0-9]+,' "$fdir/accepted.rs" |
    sed -E 's/^[[:space:]]*\(accepted, ([a-z_0-9]+),.*/\1 accepted/' >> "$tmp/langf"
  awk 'NR == FNR {k[$1] = $2; next} $1 !~ /standalone/ {print $1, $2, ($2 in k ? k[$2] : "library")}' \
    "$tmp/langf" "$tmp/feat" > "$tmp/featk"
  echo "# 按 feature 种类（去重）：kind core alloc std all_sysroot"
  for kind in unstable internal incomplete accepted library; do
    printf '%s' "$kind"
    for c in core alloc std; do
      printf ' %s' "$(awk -v c="$c" -v k="$kind" '$1 == c && $3 == k' "$tmp/featk" | wc -l | tr -d ' ')"
    done
    printf ' %s\n' "$(awk -v k="$kind" '$3 == k {print $2}' "$tmp/featk" | sort -u | wc -l | tr -d ' ')"
  done
  echo "# 全部 sysroot crate 打开的语言 feature（unstable/internal/incomplete）"
  awk '$3 != "library" && $3 != "accepted" {print $2 "[" substr($3, 1, 3) "]"}' "$tmp/featk" | sort -u | tr '\n' ' '; echo
fi

echo "# 重点 feature：feature 打开它的 crate"
for f in specialization min_specialization const_trait_impl const_destruct negative_impls auto_traits \
         never_type decl_macro rustc_attrs staged_api allow_internal_unstable lang_items intrinsics \
         core_intrinsics link_llvm_intrinsics asm_experimental_arch naked_functions repr_simd simd_ffi \
         unboxed_closures fn_traits coroutines coroutine_trait f16 f128 pattern_types pattern_type_macro \
         thread_local linkage c_variadic dropck_eyepatch may_dangle extern_types no_core prelude_import \
         rustc_private abi_unadjusted unsized_fn_params deref_pure_trait derive_const const_cmp \
         super_let builtin_syntax stmt_expr_attributes cfg_target_has_atomic target_feature_inline_always \
         x86_amx_intrinsics avx512_target_feature f16c_target_feature sse4a_target_feature \
         gen_blocks async_drop optimize_attribute ub_checks; do
  cs=$(awk -v f="$f" '$2 == f && $1 !~ /standalone/ {printf "%s%s ", $1, ($3 == "cfg" ? "(cfg)" : "")}' "$tmp/feat")
  echo "$f: ${cs:--}"
done

echo "# 语言构造出现次数（文本近似）：construct core alloc std"
count() {
  local re=$1
  for c in core alloc std; do
    if [ "$c" = core ]; then
      dirs="$lib/core/src $lib/stdarch/crates/core_arch/src/x86 $lib/stdarch/crates/core_arch/src/x86_64 $lib/portable-simd/crates/core_simd/src"
    else dirs=$lib/$c/src; fi
    # shellcheck disable=SC2086
    find $dirs -name '*.rs' | grep -v -E '/tests?(/|\.rs$)|/benches/' |
      xargs grep -hE -e "$re" 2>/dev/null | grep -v -E '^[[:space:]]*//' | wc -l | tr -d ' '
  done | tr '\n' ' '
}
while IFS='|' read -r label re; do
  echo "$label $(count "$re")"
done <<'EOF'
impl_const|impl(<[^>]*>)? const [A-Z]
tilde_or_bracket_const|\[const\] |~const |const Destruct
const_trait_decl|const (unsafe )?trait |#\[const_trait\]
default_fn|^[[:space:]]*(pub )?default (unsafe )?fn |default type
negative_impl|impl(<[^>]*>)? ![A-Z]
auto_trait|auto trait
never_type_in_sig|-> !( |$|;|\{)|<!>|for !
decl_macro|^[[:space:]]*(pub(\([a-z]+\))? )?macro [a-z_]+
macro_rules|macro_rules!
asm_macro|asm!\(
global_asm|global_asm!\(
naked_asm|naked_asm!\(
link_name_llvm|link_name = "llvm\.
extern_unadjusted|extern "unadjusted"
repr_simd|repr\(simd|repr\(C, simd
pattern_type|pattern_type!
thread_local_attr|#\[thread_local\]
linkage_attr|#\[linkage
track_caller|#\[track_caller\]
target_feature_attr|#\[target_feature\(
may_dangle|#\[may_dangle\]
coroutine_attr|#\[coroutine\]
super_let|super let
f16_f128|\bf(16|128)\b
EOF
if [ "$mode" = list ]; then
  echo "# 完整列表：crate feature cond"
  sort "$tmp/feat"
fi
