#!/usr/bin/env bash
# Phase breakdown of one build's rustc self-profile summaries.
# Usage: phase-breakdown.sh <summaries.json> [crate1,crate2,...]
#   summaries.json: { "<crate>-<pid>": { "<label>": {time, self_time, calls} } }, i.e. every rustc
#   process of a Cargo build reduced with `summarize summarize --json` (events generic-activity,query-provider).
#   The optional crate list restricts the sum to those crates (names as in the keys, without -pid).
# Prints a markdown table: summed self time per category, share of all work, share of the front end.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
[ $# -ge 1 ] || { echo "usage: $0 <summaries.json> [crates]" >&2; exit 2; }
jq -L "$here" -r --arg crates "${2:-}" -f "$here/phase-categories.jq" "$1" | jq -r -f "$here/phase-table.jq"
