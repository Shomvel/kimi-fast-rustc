#!/usr/bin/env bash
# Self-profile a clean `cargo check` with a local toolchain (relative shares only; wall intervals on a
# loaded machine are not timings). Used with a nightly whose borrowck has finer events
# (borrowck_dataflow_*, borrowck_liveness_*) than 1.94.1.
# Usage: local-check-profile.sh <cargo-project-dir> <out-dir> <rustc> <summarize> [jobs=4]
# Output: <out>/summaries.json in the per-process format of summaries-merge.jq, plus
# <out>/borrowck-split.md (mir_borrowck and its sub-events, from borrowck-split.jq).
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
[ $# -ge 4 ] || { echo "usage: $0 <project> <out> <rustc> <summarize> [jobs]" >&2; exit 2; }
proj=$1; out=$2; rustc=$3; summ=$4; jobs=${5:-4}
mkdir -p "$out/profiles"
out=$(cd "$out" && pwd)
"$rustc" -vV > "$out/rustc-version.txt"
(cd "$proj" && git rev-parse HEAD 2>/dev/null || true) > "$out/source-commit.txt"
( cd "$proj" && RUSTC="$rustc" RUSTC_BOOTSTRAP=1 CARGO_INCREMENTAL=0 CARGO_TARGET_DIR="$out/target" \
    RUSTFLAGS="-Zself-profile=$out/profiles -Zself-profile-events=default" \
    cargo check --locked -j "$jobs" ) > "$out/build.log" 2>&1
( cd "$out/profiles" && for f in *.mm_profdata; do "$summ" summarize --json "$f" > /dev/null; done )
jq -c -n -f "$here/summaries-merge.jq" "$out"/profiles/*.json > "$out/summaries.json"
jq -r -f "$here/borrowck-split.jq" "$out/summaries.json" | tee "$out/borrowck-split.md"
