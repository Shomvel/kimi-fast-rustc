#!/usr/bin/env bash
# 内建宏：library 用 #[rustc_builtin_macro] 声明了哪些宏、编译器 register_builtin_macros 登记了哪些，
# 以及 core/alloc/std 源码直接调用各内建 bang 宏、派生宏的次数（注释行与 tests/benches 不计；
# 宏模板里的出现也计入，是上界）。
#
# 用法：builtin-macros.sh LIBRARY_DIR COMPILER_DIR
set -euo pipefail
lib=${1:?usage: builtin-macros.sh LIBRARY_DIR COMPILER_DIR}
comp=${2:?usage: builtin-macros.sh LIBRARY_DIR COMPILER_DIR}
tmp=$(mktemp -d)
trap 'rm -r "$tmp"' EXIT

# library 侧：#[rustc_builtin_macro] 或 #[rustc_builtin_macro(Name, ...)] 之后的第一个宏名
find "$lib/core/src" "$lib/std/src" "$lib/alloc/src" -name '*.rs' | xargs awk '
  /^[[:space:]]*\/\// { next }
  /#\[rustc_builtin_macro/ {
    pend = 1
    if (match($0, /rustc_builtin_macro\([A-Za-z_]+/)) { n = substr($0, RSTART + 20, RLENGTH - 20); print n; pend = 0 }
    next
  }
  pend && match($0, /(macro_rules! |macro )[A-Za-z_]+/) {
    n = substr($0, RSTART, RLENGTH); sub(/.* /, "", n); print n; pend = 0
  }' | sort -u > "$tmp/lib"

# 编译器侧：register_builtin_macros 中 bang/attr/derive 三组的名字
awk '/fn register_builtin_macros/,/^}/' "$comp/rustc_builtin_macros/src/lib.rs" > "$tmp/reg"
group() { awk -v g="$1" '$0 ~ g "! *\\{" {on = 1; next} on && /^[[:space:]]*\}/ {on = 0} on && match($0, /^[[:space:]]+[A-Za-z_0-9]+:/) {
  n = substr($0, RSTART, RLENGTH - 1); gsub(/[[:space:]]/, "", n); print n }' "$tmp/reg"; }
group register_bang > "$tmp/bang"
group register_attr > "$tmp/attr"
group register_derive > "$tmp/derive"
echo "# 编译器登记：bang $(wc -l < "$tmp/bang" | tr -d ' ')，attr $(wc -l < "$tmp/attr" | tr -d ' ')，derive $(wc -l < "$tmp/derive" | tr -d ' ')"
echo "# library 用 #[rustc_builtin_macro] 声明的宏: $(wc -l < "$tmp/lib" | tr -d ' ')"
tr '\n' ' ' < "$tmp/lib"; echo
cat "$tmp/bang" "$tmp/attr" "$tmp/derive" | sort -u > "$tmp/all"
echo "# 编译器登记但 library 未声明（多为只在编译器内部使用或名字不同）"
comm -13 "$tmp/lib" "$tmp/all" | tr '\n' ' '; echo

echo "# core/alloc/std 直接调用的内建 bang 宏：macro core alloc std"
for c in core alloc std; do
  find "$lib/$c/src" -name '*.rs' | grep -v -E '/(tests|benches)/|/tests\.rs$' |
    xargs grep -hoE '(^|[^A-Za-z0-9_])[a-z_][a-z0-9_]*! *[(\[{]' 2>/dev/null |
    sed -E 's/^[^a-z_]//; s/! *[(\[{]$//' | sort | uniq -c | awk -v c="$c" '{print c, $2, $1}'
done > "$tmp/calls"
awk 'NR == FNR {b[$1] = 1; next} ($2 in b) {v[$2, $1] = $3; m[$2] = 1}
     END {for (x in m) print x, v[x, "core"] + 0, v[x, "alloc"] + 0, v[x, "std"] + 0}' "$tmp/bang" "$tmp/calls" |
  sort -k2,2nr -k4,4nr

echo "# 派生宏使用次数：derive core alloc std"
for c in core alloc std; do
  find "$lib/$c/src" -name '*.rs' | grep -v -E '/(tests|benches)/|/tests\.rs$' |
    xargs grep -hoE '#\[derive(_const)?\([^]]*\)' 2>/dev/null | sed -E 's/#\[derive(_const)?\(//; s/\)$//' |
    tr ',' '\n' | sed -E 's/^[[:space:]]+|[[:space:]]+$//g' | grep -v '^$' | sort | uniq -c | awk -v c="$c" '{print c, $2, $1}'
done | awk '{v[$2, $1] = $3; m[$2] = 1} END {for (x in m) print x, v[x, "core"] + 0, v[x, "alloc"] + 0, v[x, "std"] + 0}' |
  sort -k2,2nr
