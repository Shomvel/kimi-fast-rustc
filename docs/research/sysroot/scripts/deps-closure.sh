#!/usr/bin/env bash
# 列出一个 Cargo 项目在给定目标上的依赖闭包，并区分 target（链进最终产物）与 host（构建脚本依赖、
# 过程宏及其依赖）。只读 Cargo.lock 与清单（cargo tree --locked --offline），不构建、不运行任何脚本。
#
# 输出每行：role name version proc_macro(yes/-) source_dir
#   role = target | host | both
# 注：cargo tree 对 host 依赖按执行机平台过滤；在非 Linux 机器上运行时 host 侧的平台相关依赖
#     可能与 Linux 主机略有不同（target 侧由 --target 决定，不受影响）。
#
# 用法：deps-closure.sh MANIFEST_PATH REGISTRY_SRC [TARGET] [PACKAGE] [FEATURES]
#   MANIFEST_PATH  项目的 Cargo.toml
#   REGISTRY_SRC   ~/.cargo/registry/src/<index> 目录（registry 包的源码位置）
#   TARGET         默认 x86_64-unknown-linux-gnu
#   PACKAGE        -p 参数（工作区里选包；默认用清单的根包）
#   FEATURES       --features 参数（默认不加，即默认 features）
set -euo pipefail
manifest=${1:?usage: deps-closure.sh MANIFEST_PATH REGISTRY_SRC [TARGET] [PACKAGE] [FEATURES]}
reg=${2:?usage: deps-closure.sh MANIFEST_PATH REGISTRY_SRC [TARGET] [PACKAGE] [FEATURES]}
target=${3:-x86_64-unknown-linux-gnu}
pkg=${4:-}
feats=${5:-}

args=(tree --locked --offline --manifest-path "$manifest" --target "$target" -e normal,build
      --no-dedupe --charset ascii)
[ -n "$pkg" ] && args+=(-p "$pkg")
[ -n "$feats" ] && args+=(--features "$feats")

cargo "${args[@]}" 2>/dev/null | awk -v reg="$reg" '
  {
    line = $0
    if (match(line, /\[build-dependencies\]/)) { bdep[(RSTART - 1) / 4 + 1] = 1; next }
    if (line ~ /\[dev-dependencies\]/) next
    match(line, /[A-Za-z0-9_]/); d = (RSTART - 1) / 4
    rest = substr(line, RSTART)
    split(rest, w, " "); name = w[1]; ver = w[2]; sub(/^v/, "", ver)
    pm = (rest ~ /\(proc-macro\)/) ? "yes" : "-"
    path = ""
    if (match(rest, /\(\/[^)]*\)/)) path = substr(rest, RSTART + 1, RLENGTH - 2)
    else path = reg "/" name "-" ver
    bdep[d + 1] = 0
    host = (d > 0 && (hostctx[d - 1] || bdep[d])) || pm == "yes"
    hostctx[d] = host
    key = name " " ver
    role[key] = role[key] (host ? "h" : "t")
    pmk[key] = pm; dir[key] = path
  }
  END {
    for (k in role) {
      r = role[k]
      out = (r ~ /h/ && r ~ /t/) ? "both" : (r ~ /h/ ? "host" : "target")
      print out, k, pmk[k], dir[k]
    }
  }' | sort -k1,1 -k2,2
