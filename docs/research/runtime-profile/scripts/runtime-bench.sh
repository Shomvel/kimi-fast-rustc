#!/usr/bin/env bash
# Output check + interleaved runtime measurement of every arm's binary.
#
# usage: runtime-bench.sh WORK PROJECT OUT [ARM...]
#
#   WORK/bin/<ARM>/<bin> from build-arms.sh; workloads from workloads.sh.
#   OUT.check.tsv  workload, arm, exit, sha256 of normalized stdout+stderr, same-as-A
#   OUT.tsv        workload, arm, rep, wall_ns, instructions_u, cycles_u
#
# Each workload: one output check per arm, one warm-up round, then `reps` rounds;
# arm order alternates forward/backward between rounds (ABC CBA ...). Counters come
# from `perf stat` (user-mode instructions and cycles of the whole process tree); "NA" marks a
# counter perf did not report. perf 6.8's user_time/system_time tool events drop one of the two
# in most runs, so kernel time is measured separately by kernel-share.sh.
set -uo pipefail
WORK=${1:?usage: runtime-bench.sh WORK PROJECT OUT [ARM...]}
PROJECT=${2:?}
OUT=${3:?}
shift 3
HERE=$(cd "$(dirname "$0")" && pwd)
export RUSTUP_HOME=${RUSTUP_HOME:-/opt/rustup} CARGO_HOME=${CARGO_HOME:-/opt/cargo}
export PATH=$CARGO_HOME/bin:$PATH CARGO_TERM_COLOR=never
BIN=$([[ $PROJECT == rg ]] && echo rg || echo cargo)
ARMS=("$@")
if [[ ${#ARMS[@]} -eq 0 ]]; then
    for a in $(bash "$HERE/arm-flags.sh" --list); do [[ -x $WORK/bin/$a/$BIN ]] && ARMS+=("$a"); done
fi
bash "$HERE/workloads.sh" "$PROJECT" "$WORK" --prepare
mapfile -t ROWS < <(bash "$HERE/workloads.sh" "$PROJECT" "$WORK")
normalize() {
    case $1 in
        sort) LC_ALL=C sort ;;
        json-elapsed) sed -E 's/"elapsed(_total)?":\{[^}]*\}/"elapsed":null/g' ;;
        cargo-time) sed -E 's/ in [0-9.]+m?s$/ in <t>/' ;;
        *) cat ;;
    esac
}
printf 'workload\tarm\texit\tsha256\tsame_as_%s\n' "${ARMS[0]}" > "$OUT.check.tsv"
printf 'workload\tarm\trep\twall_ns\tinstructions_u\tcycles_u\n' > "$OUT.tsv"
STAT=$(mktemp)
for row in "${ROWS[@]}"; do
    IFS=$'\t' read -r -a f <<< "$row"
    name=${f[0]} reps=${f[1]} norm=${f[2]} cwd=${f[3]}
    args=("${f[@]:4}")
    ref=
    for arm in "${ARMS[@]}"; do
        out=$(cd "$cwd" && "$WORK/bin/$arm/$BIN" "${args[@]}" 2>&1; echo "exit=$?")
        rc=${out##*exit=}
        sha=$(printf '%s' "${out%exit=*}" | normalize "$norm" | sha256sum | cut -c1-16)
        [[ -n $ref ]] || ref=$sha
        printf '%s\t%s\t%s\t%s\t%s\n' "$name" "$arm" "$rc" "$sha" "$([[ $sha == "$ref" ]] && echo yes || echo NO)" >> "$OUT.check.tsv"
    done
    for ((rep = 0; rep <= reps; rep++)); do
        order=("${ARMS[@]}")
        if ((rep % 2)); then
            order=(); for ((i = ${#ARMS[@]} - 1; i >= 0; i--)); do order+=("${ARMS[i]}"); done
        fi
        for arm in "${order[@]}"; do
            t0=${EPOCHREALTIME/./}
            (cd "$cwd" && perf stat -x, -o "$STAT" -e instructions:u,cycles:u \
                -- "$WORK/bin/$arm/$BIN" "${args[@]}" >/dev/null 2>&1)
            t1=${EPOCHREALTIME/./}
            ((rep == 0)) && continue   # warm-up round
            counts=$(awk -F, '$1 ~ /^[0-9]+$/ {v[$3] = $1}
                END {n = split("instructions:u cycles:u", k, " ")
                     for (i = 1; i <= n; i++) printf "\t%s", (k[i] in v) ? v[k[i]] : "NA"}' "$STAT")
            printf '%s\t%s\t%s\t%s%s\n' "$name" "$arm" "$rep" "$(((t1 - t0) * 1000))" "$counts" >> "$OUT.tsv"
        done
    done
    echo "done $name ($(grep -c "^$name	" "$OUT.check.tsv") checks, mismatches: $(awk -F'\t' -v n="$name" '$1 == n && $5 == "NO" {print $2}' "$OUT.check.tsv" | paste -sd, -))"
done
rm -f "$STAT"
