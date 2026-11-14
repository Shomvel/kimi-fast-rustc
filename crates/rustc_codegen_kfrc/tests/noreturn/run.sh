#!/usr/bin/env bash
set -euo pipefail
backend=$(realpath "${1:?backend library}")
work=${2:?fresh output directory}
here=$(cd "$(dirname "$0")" && pwd)
if [ -e "$work" ]; then
    printf 'result directory already exists: %s\n' "$work" >&2
    exit 2
fi
mkdir -p "$work"
work=$(realpath "$work")
export RUSTC_BOOTSTRAP=1
python3 "$here/run.py" "$backend" "$work" "${3:-require-pruning}"
