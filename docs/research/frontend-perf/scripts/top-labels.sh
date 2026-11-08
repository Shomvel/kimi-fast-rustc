#!/usr/bin/env bash
# Largest self-profile labels of one build.
# Usage: top-labels.sh <summaries.json> [N=25] [category or ""] [crate1,crate2,...]
# Prints markdown rows: label, category (phase-lib.jq), summed self seconds, calls.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
[ $# -ge 1 ] || { echo "usage: $0 <summaries.json> [N] [category] [crates]" >&2; exit 2; }
echo "| label | 类别 | self 秒 | 调用次数 |"
echo "|---|---|---:|---:|"
jq -L "$here" -r --argjson n "${2:-25}" --arg cat "${3:-}" --arg crates "${4:-}" -f "$here/top-labels.jq" "$1"
