#!/usr/bin/env bash
# 依赖闭包中 async/await 的逐文件使用点（target 与 both 侧；源码只做文本读取）。
# 计数行：含 `async fn`/`async move`/`async {`/`async unsafe fn`/`async |`/`.await` 的非注释行。
# 每个文件附两项提示，供人工按 cfg 核对是否真的编译：
#   maybe_async=N  文件中 maybe_async 属性的出现次数（maybe-async 开 is_sync 时会把 async 改写成同步）
#   path_async     路径含 async（多为 cfg(feature = "async-*") 下的模块）
# 文本匹配不区分 cfg，是上界。
#
# 用法：async-sites.sh CLOSURE_FILE
set -euo pipefail
closure=${1:?usage: async-sites.sh CLOSURE_FILE}
re='^[^/]*(\basync (fn|move|unsafe fn|\{|\|)|\.await\b)'
tmp=$(mktemp)
trap 'rm "$tmp"' EXIT
echo "# crate ver file count [提示]"
while read -r role name ver pm dir <&3; do
  [ "$role" = host ] && continue
  base=$dir; [ -d "$dir/src" ] && base=$dir/src
  find "$base" -name '*.rs' 2>/dev/null | grep -v -E '/(tests|benches|examples|fuzz|test)/' |
    { xargs grep -cHE "$re" 2>/dev/null || true; } | { grep -v ':0$' || true; } |
    while IFS=: read -r f n; do
      rel=${f#"$dir"/}
      m=$(grep -c 'maybe_async' "$f" || true)
      hint="maybe_async=$m"
      echo "$rel" | grep -q async && hint="$hint path_async"
      echo "$name $ver $rel $n [$hint]"
    done
done 3< "$closure" | tee "$tmp"
echo "# 合计：crate 数 文件数 使用行数"
awk '{c[$1]++; f++; s += $4} END {printf "%d %d %d\n", length(c), f, s}' "$tmp"
echo "# 按 crate：crate ver 文件数 使用行数"
awk '{k = $1 " " $2; f[k]++; s[k] += $4} END {for (k in f) print k, f[k], s[k]}' "$tmp" | sort
