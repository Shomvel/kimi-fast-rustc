#!/usr/bin/env bash
# 把标准输入的 TSV（首行为表头）转成 Markdown 表格。数据列全是数字（或空、—）时右对齐，否则左对齐。
# 文档中的数字表都由本脚本从测量输出直接生成，不手抄。
# 用法：... | tsv-to-md.sh
set -euo pipefail
tab=$(printf '\t')
tmp=$(mktemp)
trap 'rm -f "$tmp"' EXIT
grep -v '^$' > "$tmp" || true
[ -s "$tmp" ] || exit 0
n=$(head -1 "$tmp" | tr -cd "$tab" | wc -c | tr -d ' ')
n=$((n + 1))
sepline='|'
i=1
while [ "$i" -le "$n" ]; do
  if tail -n +2 "$tmp" | cut -f"$i" | grep -qvE '^[-+0-9.,%/ —]*$'; then
    sepline="$sepline---|"
  else
    sepline="$sepline---:|"
  fi
  i=$((i + 1))
done
first=1
while IFS= read -r line; do
  printf '| %s |\n' "$(printf '%s' "$line" | sed "s/$tab/ | /g")"
  if [ "$first" = 1 ]; then
    printf '%s\n' "$sepline"
    first=0
  fi
done < "$tmp"
