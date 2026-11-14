#!/usr/bin/env bash
# Compare two flat profiles of the same workload (profile-hotspots.sh *.sym.txt).
#
# usage: compare-profiles.sh BASE.sym.txt ARM.sym.txt RATIO [TOPN]
#
#   RATIO  cycles(arm)/cycles(base) for this workload (from summarize-runtime.sh), used to
#          express the arm's per-symbol share in units of the base run's total cycles
#   Prints the TOPN (default 10) symbols by absolute change: base %, arm % (own run),
#   arm % scaled by RATIO, and the difference in base-run percentage points.
set -euo pipefail
B=${1:?usage: compare-profiles.sh BASE.sym.txt ARM.sym.txt RATIO [TOPN]}
A=${2:?}
R=${3:?}
N=${4:-10}
# Symbols are normalized so that the same function matches across builds: legacy hash
# (17h…E), ".llvm.NNN" suffixes and symbol versions (@GLIBC_…) are dropped, then summed.
parse() {
    sed -E 's/^ *([0-9.]+)% +([^ ]+) +\[[.k]\] +/\1\t\2 /; s/17h[0-9a-f]{16}E//; s/::h[0-9a-f]{16}//;
            s/\.llvm\.[0-9]+//; s/@+GLIBC_[0-9.]+//' "$1" \
        | awk -F'\t' 'NF == 2 {sum[$2] += $1} END {for (k in sum) print k "\t" sum[k]}'
}
echo "| symbol | base % | arm % | arm % × ratio | Δ (pp of base) |"
echo "|---|---:|---:|---:|---:|"
join -t $'\t' -a1 -a2 -e 0 -o 0,1.2,2.2 <(parse "$B" | sort -t $'\t' -k1,1) <(parse "$A" | sort -t $'\t' -k1,1) \
    | awk -F'\t' -v r="$R" '{s = $3 * r; d = s - $2; printf "%s\t%.2f\t%.2f\t%.2f\t%+.2f\t%.4f\n", $1, $2, $3, s, d, (d < 0 ? -d : d)}' \
    | sort -t $'\t' -k6,6gr | head -n "$N" \
    | awk -F'\t' '{printf "| %s | %s | %s | %s | %s |\n", substr($1, 1, 100), $2, $3, $4, $5}'
