#!/usr/bin/env bash
# Old vs next trait solver on a clean `cargo check`, ABBA, by instructions retired summed over all
# rustc processes (macOS `time -l` per process via time-rustc-wrapper.sh; robust to machine load,
# unlike wall time).
# Usage: next-solver-ab.sh <cargo-project-dir> <out-dir> <rustc> [jobs=4]
#   A = default (1.94.1: next solver only in coherence), B = -Znext-solver=globally.
# Output: <out>/<n>-<arm>/{build.log,rustc-*.txt}, and <out>/summary.md (one row per run).
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
[ $# -ge 3 ] || { echo "usage: $0 <project> <out> <rustc> [jobs]" >&2; exit 2; }
proj=$1; out=$2; rustc=$3; jobs=${4:-4}
mkdir -p "$out"; out=$(cd "$out" && pwd)
"$rustc" -vV > "$out/rustc-version.txt"
{ echo "| 次序 | 臂 | instructions retired（全部 rustc） | rustc 进程 | 退出码 |"; echo "|---:|---|---:|---:|---:|"; } > "$out/summary.md"
n=0
for arm in A B B A; do
  n=$((n + 1)); d="$out/$n-$arm"; mkdir -p "$d"; rm -rf "$out/target"
  flags=""; [ "$arm" = B ] && flags="-Znext-solver=globally"
  status=0
  ( cd "$proj" && RUSTC="$rustc" RUSTC_BOOTSTRAP=1 CARGO_INCREMENTAL=0 CARGO_TARGET_DIR="$out/target" \
      RUSTFLAGS="$flags" RUSTC_WRAPPER="$here/time-rustc-wrapper.sh" KFRC_TIME_DIR="$d" cargo check --locked -j "$jobs" ) > "$d/build.log" 2>&1 || status=$?
  instr=$(grep -h "instructions retired" "$d"/rustc-*.txt | sed "s/^ *\([0-9]*\).*/\1/" | paste -sd+ - | bc)
  procs=$(ls "$d"/rustc-*.txt | wc -l | tr -d " ")
  echo "| $n | $arm | $instr | $procs | $status |" >> "$out/summary.md"
done
rm -rf "$out/target"
cat "$out/summary.md"
