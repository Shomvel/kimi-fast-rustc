#!/usr/bin/env bash
# suite-runtime.sh <results-root> <arm> [<arm> ...]
# Serial interleaved timing of every arm's LLVM driver (warm-up + SUITE_RUNTIME_ROUNDS, default 11),
# then one perf-stat profile per arm/workload. Run on an otherwise idle sandbox.
set -euo pipefail
root=${1:?results root}; shift
here=$(cd "$(dirname "$0")" && pwd)
python3 -I "$here/suite.py" runtime "$root" "$@"
