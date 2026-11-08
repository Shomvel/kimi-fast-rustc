#!/usr/bin/env bash
# 从各测量脚本的输出生成文档用的 Markdown 表（WORKDIR/tables/*.md）。
# 前置：pipelines.sh WORKDIR/pipelines、codegen-passes.sh WORKDIR/cg、cargo-flags.sh WORKDIR/cargo 已运行；
# CENSUS_DIR 为前期 E2B 诊断结果目录（含 census-rg-A.json、census-rg-Z.json、census-cargo-Z.json）。
# 用法：make-tables.sh WORKDIR CENSUS_DIR
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
work=${1:?usage: make-tables.sh WORKDIR CENSUS_DIR}
census=${2:?usage: make-tables.sh WORKDIR CENSUS_DIR}
md=$here/tsv-to-md.sh
t=$work/tables
mkdir -p "$t" "$work/census"
tab=$(printf '\t')

# 流水线矩阵与 pass 计数
p=$work/pipelines/out
grep -E '^(config|O3 |O0 cgu=16 lto=default|O1 cgu=16 lto=default|O2 cgu=16 lto=default|Os cgu=16 lto=default|Oz cgu=16 lto=default)' \
  "$p/matrix.tsv" | "$md" > "$t/matrix-sel.md"
cut -f1,6,7,8,9,10 "$p/pass-table.tsv" | "$md" > "$t/pass-table-o3.md"
grep -E "^(pass|leaf-total|sroa|instcombine|simplifycfg|early-cse|gvn|inline|argpromotion|jump-threading|correlated-propagation|dse|memcpyopt|licm|loop-unroll|loop-vectorize|slp-vectorizer|mergefunc)$tab" "$p/pass-table.tsv" \
  | cut -f1,2,3,4,5,6,7,8,11,12 | "$md" > "$t/pass-table-levels.md"

# 后端
cut -f1,2,3,5,6 "$work/cg/codegen/summary.tsv" | "$md" > "$t/codegen-summary.md"

# E2B 逐 pass 重放
for n in rg-A rg-Z cargo-Z; do
  "$here/census-passes.sh" "$census/census-$n.json" 25 > "$work/census/$n.tsv"
done
sec() { sed -n "/^## $2/,/^\$/p" "$work/census/$1.tsv" | sed 's/^## //'; }
for n in rg-A rg-Z cargo-Z; do
  sec "$n" stage | "$md" > "$t/stage-$n.md"
  sec "$n" family | "$md" > "$t/family-$n.md"
  sec "$n" top | head -16 | "$md" > "$t/top-$n.md"
  sec "$n" codegen | "$md" > "$t/cgfam-$n.md"
done
fam() { sec "$1" family | tail -n +2 | grep . | cut -f1,6 | sort -t"$tab" -k1,1; }
join -t"$tab" <(fam rg-A) <(fam rg-Z) | join -t"$tab" - <(fam cargo-Z) | sort -t"$tab" -k2,2 -rn \
  | { printf 'family\trg A %%\trg Z %%\tcargo Z %%\n'; cat; } | "$md" > "$t/family-compare.md"
stg() { sec "$1" stage | tail -n +2 | grep . | cut -f1,2,4 | sort -t"$tab" -k1,1; }
join -t"$tab" <(stg rg-A) <(stg rg-Z) | join -t"$tab" - <(stg cargo-Z) \
  | { printf 'stage\trg A 进程 CPU s\trg A 计时占比 %%\trg Z 进程 CPU s\trg Z 计时占比 %%\tcargo Z 进程 CPU s\tcargo Z 计时占比 %%\n'; cat; } \
  | "$md" > "$t/stage-compare.md"

# cargo 参数
grep -A4 -E '^## profile=release lto=(default|thin)' "$work/cargo/cargo-flags.tsv" > "$t/cargo-flags-release.txt" || true
echo "done: $t"

# README 总表：families.tsv 的定性列 + 静态次数（counts.tsv）+ 计时占比（census 族表）
share_of() { sec "$1" family | tail -n +2 | grep . | grep "^$2$tab" | cut -f6 || true; }
count_sum() { # LABEL "PASS..."
  local s=0 c
  for ps in $2; do
    c=$(grep "^$1$tab$ps$tab" "$p/counts.tsv" | cut -f3 || true)
    s=$((s + ${c:-0}))
  done
  echo "$s"
}
{
  printf '族\t类\t位置\tpre/post 次数\trg A %%\tcargo Z %%\t做什么\tkfrc 承接\n'
  grep -v '^#' "$here/families.tsv" | while IFS="$tab" read -r fam passes cls pos what kfrc; do
    if [ "$passes" = "-" ]; then cnt="—"; else cnt="$(count_sum ThinPre-O3 "$passes")/$(count_sum ThinPost-O3 "$passes")"; fi
    printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' "$fam" "$cls" "$pos" "$cnt" "$(share_of rg-A "$fam")" "$(share_of cargo-Z "$fam")" "$what" "$kfrc"
  done
} | "$md" > "$t/master.md"
echo "done: $t/master.md"
