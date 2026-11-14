#!/usr/bin/env bash
# Upper bound for a cross-crate shared cache of canonical trait/type-op goals.
# Usage: trait-goal-dup.sh <cargo-project-dir> <out-dir> <rustc> <crox> [jobs=4]
#   <rustc> must accept -Z flags (RUSTC_BOOTSTRAP=1 is set). The build is `cargo check` with
#   -Zself-profile-events=default,query-keys; every process's profile is converted with crox,
#   events are reduced to self time per (query, canonical key, crate), and goal-dup.awk counts
#   keys computed in more than one crate. rustc's caches are per session, so each such key was
#   evaluated again in every crate; a shared cache could save all but one evaluation.
# Caveats: keys are compared as printed strings (generic parameters are printed by name, so two
# different functions' `T: Clone` environments compare equal); self times are wall intervals.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
[ $# -ge 4 ] || { echo "usage: $0 <project> <out> <rustc> <crox> [jobs]" >&2; exit 2; }
proj=$1; out=$2; rustc=$3; crox=$4; jobs=${5:-4}
mkdir -p "$out/profiles" "$out/tmp"
out=$(cd "$out" && pwd)
"$rustc" -vV > "$out/rustc-version.txt"
(cd "$proj" && git rev-parse HEAD 2>/dev/null || true) > "$out/source-commit.txt"
shasum -a 256 "$proj/Cargo.lock" > "$out/cargo-lock.sha256"
( cd "$proj" && RUSTC="$rustc" RUSTC_BOOTSTRAP=1 CARGO_INCREMENTAL=0 CARGO_TARGET_DIR="$out/target" \
    RUSTFLAGS="-Zself-profile=$out/profiles -Zself-profile-events=default,query-keys" \
    cargo check --locked -j "$jobs" ) > "$out/build.log" 2>&1
: > "$out/goals.tsv"
for f in "$out"/profiles/*.mm_profdata; do
  crate=$(basename "$f" .mm_profdata | sed 's/-[0-9]*$//')
  ( cd "$out/tmp" && "$crox" "$f" > /dev/null )
  jq -r --arg crate "$crate" -f "$here/goal-events.jq" "$out/tmp/chrome_profiler.json" \
    | sort -t "$(printf '\t')" -k1,1 -k2,2n -k3,3nr | awk -f "$here/goal-self-time.awk" >> "$out/goals.tsv"
  rm -f "$out/tmp/chrome_profiler.json"
done
awk -f "$here/goal-dup.awk" "$out/goals.tsv" | tee "$out/goal-dup.md"
