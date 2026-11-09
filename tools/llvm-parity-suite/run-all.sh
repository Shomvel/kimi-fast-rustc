#!/usr/bin/env bash
# run-all.sh <results-root> <arm>=<backend|llvm> ... : every arm serially, then interleaved runtime, then the table.
# Example: run-all.sh /workspace/suite-results/20261110 llvm=llvm c10=/workspace/backends/c10.so perm=/workspace/backends/perm.so
set -euo pipefail
root=${1:?results root}; shift
here=$(cd "$(dirname "$0")" && pwd)
arms=()
for spec in "$@"; do
    name=${spec%%=*}; backend=${spec#*=}
    bash "$here/suite-run.sh" "$name" "$backend" "$root" 2>&1 | tee "$root.$name.log"
    arms+=("$name")
done
bash "$here/suite-runtime.sh" "$root" "${arms[@]}" 2>&1 | tee "$root.runtime.log"
python3 -I "$here/gate_table.py" "$root" ${SUITE_TABLE_ARGS:-} > /dev/null
echo "table: $root/table.md"
