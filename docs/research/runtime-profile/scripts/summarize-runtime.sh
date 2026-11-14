#!/usr/bin/env bash
# Summarize runtime-bench.sh output: per workload, each arm's median relative to the base arm.
#
# usage: summarize-runtime.sh BENCH.tsv METRIC [BASE_ARM] [ARMS]
#
#   METRIC  cycles_u | instructions_u | wall_ns
#   ARMS    comma-separated arms to show, in order (default: all, in file order)
#   Prints a Markdown table: workload, base median (absolute), base spread
#   ((max-min)/median over reps), then median(arm)/median(base). "NA" samples are ignored;
#   a cell is "–" when an arm has no valid sample or the base median is 0.
set -euo pipefail
TSV=${1:?usage: summarize-runtime.sh BENCH.tsv METRIC [BASE_ARM] [ARMS]}
METRIC=${2:?}
BASE=${3:-A}
SHOW=${4:-}
awk -F'\t' -v metric="$METRIC" -v base="$BASE" -v show="$SHOW" '
function median(key,   n, i, j, t, a) {
    n = split(vals[key], a, " ")
    for (i = 2; i <= n; i++) { t = a[i]; for (j = i - 1; j >= 1 && a[j] + 0 > t + 0; j--) a[j + 1] = a[j]; a[j + 1] = t }
    lo = a[1]; hi = a[n]
    return n % 2 ? a[(n + 1) / 2] : (a[n / 2] + a[n / 2 + 1]) / 2
}
NR == 1 { for (i = 1; i <= NF; i++) col[$i] = i; next }
{
    w = $col["workload"]; arm = $col["arm"]; v = $col[metric]
    if (!(w in seenw)) { seenw[w] = 1; ws[++nw] = w }
    if (!(arm in seena)) { seena[arm] = 1; as[++na] = arm }
    if (v != "NA" && v != "") vals[w SUBSEP arm] = vals[w SUBSEP arm] " " v
}
END {
    if (show != "") na = split(show, as, ",")
    printf "| workload | %s %s | spread |", base, metric
    for (k = 1; k <= na; k++) if (as[k] != base) printf " %s |", as[k]
    printf "\n|---|---:|---:|"
    for (k = 1; k <= na; k++) if (as[k] != base) printf "---:|"
    printf "\n"
    for (i = 1; i <= nw; i++) {
        w = ws[i]
        b = ((w SUBSEP base) in vals) ? median(w SUBSEP base) : 0
        spread = b > 0 ? (hi - lo) / b : 0
        unit = metric ~ /_ns$/ ? sprintf("%.2f ms", b / 1e6) : sprintf("%.4fG", b / 1e9)
        printf "| %s | %s | %.1f%% |", w, unit, 100 * spread
        for (k = 1; k <= na; k++) {
            a = as[k]; if (a == base) continue
            if (!((w SUBSEP a) in vals) || b == 0) { printf " – |"; continue }
            printf " %.3f |", median(w SUBSEP a) / b
        }
        printf "\n"
    }
}' "$TSV"
