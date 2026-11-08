#!/usr/bin/env bash
# Markdown table of the profiled scenarios (wall, CPU, instructions:u; self-profile overhead included).
# Usage: scenario-table.sh <results/profile>   (directory with <project>/<scenario>/result.json)
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
[ $# -eq 1 ] || { echo "usage: $0 <profile-dir>" >&2; exit 2; }
jq -s -r -f "$here/scenario-md.jq" "$1"/*/*/result.json
