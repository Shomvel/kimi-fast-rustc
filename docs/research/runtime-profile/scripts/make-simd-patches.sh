#!/usr/bin/env bash
# Make patched copies of memchr and aho-corasick that hide explicit-SIMD paths.
#
# usage: make-simd-patches.sh REGISTRY_SRC LOCKFILE OUT
#
#   REGISTRY_SRC  Cargo registry source dir (…/registry/src/index.crates.io-*)
#   LOCKFILE      project Cargo.lock; selects the memchr / aho-corasick versions
#   OUT           writes OUT/S2/{memchr,aho-corasick} and OUT/S0/{memchr,aho-corasick}
#
# S2: runtime AVX2 detection answers "no" -> memchr uses SSE2, Teddy uses SSSE3.
# S0: additionally SSSE3 detection and memchr's SSE2 `is_available` answer "no"
#     -> memchr/memmem fall back to SWAR / Two-Way, aho-corasick has no Teddy.
# Only `is_available`-style dispatch is changed; algorithms are untouched.
# hashbrown's SSE2 group (compile-time cfg, also in the prebuilt std) is not patched.
set -euo pipefail
REG=${1:?usage: make-simd-patches.sh REGISTRY_SRC LOCKFILE OUT}
LOCK=${2:?}
OUT=${3:?}
ver() { awk -v n="$1" '$0 == "name = \"" n "\"" {getline; gsub(/version = |"/, ""); print; exit}' "$LOCK"; }
sse2_true() {  # is_available() bodies that still return true under cfg(sse2)
    find "$1/src" -name '*.rs' ! -name 'tests.rs' -print0 | xargs -0 perl -0ne \
        '$n += () = /fn is_available\(\) -> bool \{(?:(?!\bfn\b).){0,300}?#\[cfg\(target_feature = "sse2"\)\]\s*\{\s*true/gs; END { print $n + 0 }'
}
count() { grep -rF "$1" "$2/src" --include=*.rs | grep -v '/tests\?\.rs:' | wc -l; }
for arm in S2 S0; do
    for crate in memchr aho-corasick; do
        v=$(ver "$crate")
        [[ -n $v ]] || { echo "$crate not in $LOCK" >&2; exit 1; }
        d=$OUT/$arm/$crate
        mkdir -p "$OUT/$arm"
        [[ -d $d ]] || cp -r "$REG/$crate-$v" "$d"
        # Non-test sources only; tests keep their own detection.
        find "$d/src" -name '*.rs' ! -name 'tests.rs' -print0 \
            | xargs -0 perl -0pi -e 's/std::is_x86_feature_detected!\("avx2"\)/false/g'
        if [[ $arm == S0 ]]; then
            find "$d/src" -name '*.rs' ! -name 'tests.rs' -print0 \
                | xargs -0 perl -0pi -e 's/std::is_x86_feature_detected!\("ssse3"\)/false/g;
                    s/(fn is_available\(\) -> bool \{(?:(?!\bfn\b).){0,300}?#\[cfg\(target_feature = "sse2"\)\]\s*\{\s*)true/${1}false/gs'
        fi
        echo "$arm $crate-$v avx2_detect_left=$(count 'is_x86_feature_detected!("avx2")' "$d")" \
             "ssse3_detect_left=$(count 'is_x86_feature_detected!("ssse3")' "$d")" \
             "sse2_available_true_left=$(sse2_true "$d")" \
             "patched_files=$(diff -rq "$REG/$crate-$v/src" "$d/src" | wc -l)"
    done
done
