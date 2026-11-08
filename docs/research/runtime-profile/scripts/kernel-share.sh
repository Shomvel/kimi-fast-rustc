#!/usr/bin/env bash
# User vs kernel CPU time of one arm on every workload.
#
# usage: kernel-share.sh WORK PROJECT ARM [N]
#
# Each workload runs N times (default 20) back to back under one `/usr/bin/time`
# (whose resolution is 10 ms, hence the batching); prints per-run wall, user and
# system milliseconds and sys/(user+sys). The batch shell's own fork/exec cost is included.
set -euo pipefail
WORK=${1:?usage: kernel-share.sh WORK PROJECT ARM [N]}
PROJECT=${2:?}
ARM=${3:?}
N=${4:-20}
HERE=$(cd "$(dirname "$0")" && pwd)
export RUSTUP_HOME=${RUSTUP_HOME:-/opt/rustup} CARGO_HOME=${CARGO_HOME:-/opt/cargo}
export PATH=$CARGO_HOME/bin:$PATH CARGO_TERM_COLOR=never
BIN=$([[ $PROJECT == rg ]] && echo rg || echo cargo)
T=$(mktemp)
echo "| workload | wall ms/run | user ms/run | sys ms/run | sys/(user+sys) |"
echo "|---|---:|---:|---:|---:|"
while IFS=$'\t' read -r -a f; do
    name=${f[0]} cwd=${f[3]}
    args=("${f[@]:4}")
    (cd "$cwd" && "$WORK/bin/$ARM/$BIN" "${args[@]}" >/dev/null 2>&1) || true   # warm caches
    (cd "$cwd" && /usr/bin/time -f '%e %U %S' -o "$T" \
        bash -c 'for ((i = 0; i < $1; i++)); do "${@:2}" >/dev/null 2>&1; done' _ "$N" \
        "$WORK/bin/$ARM/$BIN" "${args[@]}")
    read -r e u s < "$T"
    awk -v w="$name" -v n="$N" -v e="$e" -v u="$u" -v s="$s" \
        'BEGIN {printf "| %s | %.2f | %.2f | %.2f | %.0f%% |\n", w, 1000 * e / n, 1000 * u / n, 1000 * s / n, (u + s) > 0 ? 100 * s / (u + s) : 0}'
done < <(bash "$HERE/workloads.sh" "$PROJECT" "$WORK")
rm -f "$T"
