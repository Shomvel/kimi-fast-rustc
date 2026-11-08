#!/usr/bin/env bash
# Front-end phase shares of several builds side by side.
# Usage: phase-matrix.sh <name>=<summaries.json> [<name>=<summaries.json> ...]
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
[ $# -ge 1 ] || { echo "usage: $0 name=summaries.json ..." >&2; exit 2; }
for spec in "$@"; do
  name=${spec%%=*}; file=${spec#*=}
  jq -L "$here" -c --arg crates "" -f "$here/phase-categories.jq" "$file" | jq -c --arg name "$name" -f "$here/phase-wrap.jq"
done | jq -s -r -f "$here/phase-matrix.jq"
