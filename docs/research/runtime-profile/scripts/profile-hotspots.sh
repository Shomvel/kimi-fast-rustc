#!/usr/bin/env bash
# Sample user-mode cycles of one arm's binary on every workload with perf.
#
# usage: profile-hotspots.sh WORK PROJECT ARM OUT_DIR
#
#   OUT_DIR/<workload>.sym.txt  flat profile by shared object + symbol (self cycles, >= 0.1%)
#   OUT_DIR/<workload>.dso.txt  flat profile by shared object
# Install libc debug symbols (libc6-dbg) first, or libc-internal functions stay as addresses.
# Only user mode is sampled (perf_event_paranoid=2); kernel share comes from
# runtime-bench.sh (user/system time). Inlined code is charged to its caller.
set -euo pipefail
WORK=${1:?usage: profile-hotspots.sh WORK PROJECT ARM OUT_DIR}
PROJECT=${2:?}
ARM=${3:?}
OUT=${4:?}
HERE=$(cd "$(dirname "$0")" && pwd)
export RUSTUP_HOME=${RUSTUP_HOME:-/opt/rustup} CARGO_HOME=${CARGO_HOME:-/opt/cargo}
export PATH=$CARGO_HOME/bin:$PATH CARGO_TERM_COLOR=never
BIN=$([[ $PROJECT == rg ]] && echo rg || echo cargo)
mkdir -p "$OUT"
bash "$HERE/workloads.sh" "$PROJECT" "$WORK" --prepare
while IFS=$'\t' read -r -a f; do
    name=${f[0]} cwd=${f[3]}
    args=("${f[@]:4}")
    data=$OUT/$name.data
    # Every workload is repeated so that each profile has thousands of samples.
    reps=10; [[ ${f[1]} -ge 20 ]] && reps=40
    (cd "$cwd" && perf record -q -e cycles:u -F 4999 -o "$data" -- \
        bash -c 'for ((i = 0; i < $1; i++)); do "${@:2}" >/dev/null 2>&1; done' _ "$reps" \
        "$WORK/bin/$ARM/$BIN" "${args[@]}" 2>/dev/null)
    perf report -i "$data" --stdio --no-children --sort dso,sym -q --percent-limit 0.1 \
        --comm "$BIN" 2>/dev/null | sed '/^$/d' > "$OUT/$name.sym.txt"
    perf report -i "$data" --stdio --no-children --sort dso -q --percent-limit 0.1 \
        2>/dev/null | sed '/^$/d' > "$OUT/$name.dso.txt"
    samples=$(perf report -i "$data" --stdio --sort comm -q --show-nr-samples 2>/dev/null | awk -v b="$BIN" '$3 == b {print $2}')
    echo "$name reps=$reps samples=$samples" | tee -a "$OUT/samples.txt"
    rm -f "$data" "$data.old"
done < <(bash "$HERE/workloads.sh" "$PROJECT" "$WORK")
