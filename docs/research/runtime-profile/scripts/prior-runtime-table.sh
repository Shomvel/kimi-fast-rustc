#!/usr/bin/env bash
# Extract runtime results of the 2026-10 E2B rounds (mirx bench_runtime.py JSON) into one table.
#
# usage: prior-runtime-table.sh RUNS_DIR [range]
#
#   RUNS_DIR  the rounds directory of the 2026-10 rustc build-profile material
#             (each round: <round>/results/<name>-runtime.json; files without per-arm
#             counters, i.e. older wall-only rounds, are skipped)
#   default:  TSV  round, file, workload, arm, A instructions, instr ratio, cycles ratio, wall ratio
#   range:    per round/file/arm, min..max of the instruction and cycle ratios over workloads (Markdown)
# Ratios are median(arm)/median(A) over the round's repetitions; A is each round's own baseline.
set -euo pipefail
DIR=${1:?usage: prior-runtime-table.sh RUNS_DIR [range]}
MODE=${2:-tsv}
rows() {
    for f in "$DIR"/*/results/*-runtime.json; do
        round=$(basename "$(dirname "$(dirname "$f")")")
        jq -r --arg round "$round" --arg file "$(basename "$f" .json)" '
            (.results // {}) | to_entries[] | .key as $w | .value.arms as $a | ($a.A // empty) as $b
            | $a | to_entries[] | select(.key != "A")
            | [$round, $file, $w, .key, $b.instructions,
               (.value.instructions / $b.instructions), (.value.cycles / $b.cycles), (.value.wall / $b.wall)]
            | @tsv' "$f"
    done
}
if [[ $MODE == range ]]; then
    echo "| round | file | arm | workloads | instructions:u ratio | cycles:u ratio |"
    echo "|---|---|---|---:|---:|---:|"
    rows | awk -F'\t' '{k = $1 "|" $2 "|" $4; n[k]++
            if (!(k in lo) || $6 < lo[k]) lo[k] = $6; if (!(k in hi) || $6 > hi[k]) hi[k] = $6
            if (!(k in clo) || $7 < clo[k]) clo[k] = $7; if (!(k in chi) || $7 > chi[k]) chi[k] = $7}
        END {for (k in n) {split(k, p, "|"); printf "| %s | %s | %s | %d | %.3f..%.3f | %.3f..%.3f |\n", p[1], p[2], p[3], n[k], lo[k], hi[k], clo[k], chi[k]}}' | sort
else
    printf 'round\tfile\tworkload\tarm\tA_instructions\tinstr_ratio\tcycles_ratio\twall_ratio\n'
    rows
fi
