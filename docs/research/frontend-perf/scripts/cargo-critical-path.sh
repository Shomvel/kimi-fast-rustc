#!/usr/bin/env bash
# Critical path of a Cargo build, split into front end (until rmeta) and codegen.
# Usage: cargo-critical-path.sh <cargo-timing.html>
# Extracts the UNIT_DATA array that `cargo build --timings` embeds in its HTML report and runs
# cargo-critical-path.jq on it.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
[ $# -eq 1 ] || { echo "usage: $0 <cargo-timing.html>" >&2; exit 2; }
awk '/const UNIT_DATA = \[/{f=1; sub(/.*const UNIT_DATA = /, "")} f{print} f && /^\];/{exit}' "$1" \
  | sed '$ s/];/]/' | jq -r -f "$here/cargo-critical-path.jq"
