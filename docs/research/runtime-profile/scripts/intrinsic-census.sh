#!/usr/bin/env bash
# List the x86 core::arch intrinsics that the hot SIMD crates call.
#
# usage: intrinsic-census.sh REGISTRY_SRC CRATE-VERSION...
#   e.g. intrinsic-census.sh ~/.cargo/registry/src/index.crates.io-* memchr-2.8.3 aho-corasick-1.1.4
#
# Per crate: number of distinct _mm*/_mm256*/_mm512* intrinsics and the list, plus the
# #[target_feature(enable = ...)] values (the dispatch targets). Test files and comment
# lines are skipped.
set -euo pipefail
REG=${1:?usage: intrinsic-census.sh REGISTRY_SRC CRATE-VERSION...}
shift
for c in "$@"; do
    d=$REG/$c/src
    [[ -d $d ]] || { echo "| $c | missing |"; continue; }
    files=$(find "$d" -name '*.rs' ! -name 'tests.rs' ! -path '*/tests/*')
    code=$(grep -hv '^[[:space:]]*//' $files)   # drop comment lines
    intr=$(grep -oE '\b_mm(256|512)?_[a-z0-9_]+\b' <<< "$code" | sort -u || true)
    feats=$( (grep -oE 'target_feature\(enable *= *"[a-z0-9.,]+"\)' <<< "$code" || true) | sed -E 's/.*"(.*)".*/\1/' \
        | sort | uniq -c | awk '{printf "%s%s×%s", sep, $2, $1; sep = ", "}')
    printf '| %s | %d | %s | %s |\n' "$c" "$(printf '%s\n' "$intr" | grep -c .)" "${feats:--}" "$(printf '%s ' $intr)"
done
