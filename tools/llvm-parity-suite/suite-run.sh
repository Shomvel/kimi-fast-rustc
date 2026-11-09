#!/usr/bin/env bash
# suite-run.sh <arm-name> <backend.so|llvm> [results-root]
# Per-arm stage: library build (formal flags), size, compile instructions (perf stat), LLVM driver,
# corpus differential, upstream tests. Writes <results-root>/<arm-name>/<crate>.json.
# Optional env: SUITE_COMPILE_ROUNDS (default 3), SUITE_CRATES=a,b (subset).
set -euo pipefail
arm=${1:?arm name}
backend=${2:?backend.so or llvm}
root=${3:-/workspace/suite-results/$(date -u +%Y%m%d)}
here=$(cd "$(dirname "$0")" && pwd)
if [ "$backend" != llvm ]; then backend=$(realpath "$backend"); fi
mkdir -p "$root"
if [ ! -f "$root/inputs.json" ]; then python3 -I "$here/suite.py" prepare "$root"; fi
python3 -I "$here/suite.py" arm "$root" "$arm" "$backend"
