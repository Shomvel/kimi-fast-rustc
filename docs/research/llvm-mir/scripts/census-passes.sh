#!/usr/bin/env bash
# 把前期 E2B 诊断（2026-10-10，Linux x86_64，8 vCPU，rustc 1.94.1 stage1 / LLVM 21）的
# 逐 pass 重放结果汇总成阶段、pass 族与单个 pass 的时间占比。重放方法见 census JSON 的生成脚本：
# pre-link `opt -passes=thinlto-pre-link<O3>`，post-link `opt -passes=thinlto<O3>`，codegen `llc -O3`，
# 每个 CGU 单进程、-time-passes。
# 用法：census-passes.sh CENSUS_JSON [TOP]
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
json=${1:?usage: census-passes.sh CENSUS_JSON [TOP]}
top=${2:-25}
jq -r --argjson top "$top" -f "$here/census-passes.jq" "$json"
