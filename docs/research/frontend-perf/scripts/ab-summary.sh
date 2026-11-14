#!/usr/bin/env bash
# Means per arm and B/A ratio from a next-solver-ab.sh summary.md (column 3 = instructions).
# Usage: ab-summary.sh <summary.md>
set -euo pipefail
[ $# -eq 1 ] || { echo "usage: $0 <summary.md>" >&2; exit 2; }
mean() { grep -E "^\| [0-9]+ \| $1 \|" "$2" | cut -d'|' -f4 | tr -d ' ' | paste -sd+ - | sed 's/^/(/; s/$/)/' \
  | sed "s|\$|/$(grep -cE "^\| [0-9]+ \| $1 \|" "$2")|" | bc; }
a=$(mean A "$1"); b=$(mean B "$1")
echo "A 均值 ${a}；B 均值 ${b}；B/A = $(echo "scale=4; ${b} / ${a}" | bc)"
