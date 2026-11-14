#!/usr/bin/env bash
# Full unchanged upstream internal unit suite, including the rustc-generated
# test harness, compiled by the selected backend. Prebuilt libtest/std remain LLVM.
# Optional phase allows one LLVM baseline followed by multiple candidate fixes.
set -euo pipefail
backend=$(realpath "${1:?backend dylib}")
work=${2:?result directory}
phase=${3:-all}
here=$(cd "$(dirname "$0")" && pwd)
mkdir -p "$work"
work=$(realpath "$work")
export RUSTC_BOOTSTRAP=1
toolchain=${KFRC_TOOLCHAIN:-1.94.1}
if [ ! -d "$work/regex-syntax-0.8.8" ]; then
    python3 "$here/regex-fetch.py" "$work" > "$work/source-path.txt"
fi
source="$work/regex-syntax-0.8.8"
python3 "$here/regex-test-config.py" "$work" > "$work/test-features.txt"
flags=(--edition=2021 --crate-name=regex_syntax --target=x86_64-unknown-linux-gnu
       --test -Cpanic=abort -Zpanic-abort-tests -Copt-level=0 -Coverflow-checks=off
       -Cdebug-assertions=off -Cdebuginfo=0 -Ccodegen-units=1 -Csave-temps=on -Zmir-opt-level=0)
while IFS= read -r feature; do flags+=(--cfg "feature=\"$feature\""); done < "$work/test-features.txt"
case "$phase" in
    all) arms=(llvm kfrc);;
    baseline) arms=(llvm);;
    candidate) test -f "$work/llvm/tests.jsonl"; arms=(kfrc);;
    *) echo "phase must be all, baseline or candidate" >&2; exit 2;;
esac
for arm in "${arms[@]}"; do
    result="$work/$arm"
    if [ -e "$result" ]; then
        echo "refusing to overwrite existing arm results: $result" >&2
        exit 2
    fi
    mkdir -p "$result"
    rustc +"$toolchain" -vV > "$result/rustc-version.txt"
    sha256sum "$backend" "$here/regex-tests.sh" "$here/regex-test-config.py" "$here/regex-tests-summary.py" > "$result/input.sha256"
    backend_flags=()
    if [ "$arm" = kfrc ]; then backend_flags=(-Zcodegen-backend="$backend"); fi
    command=(rustc +"$toolchain" "${flags[@]}" "${backend_flags[@]}" "$source/src/lib.rs" -o "$result/regex_syntax_tests")
    printf '%q ' "${command[@]}" > "$result/compile-command.sh"
    printf '\n' >> "$result/compile-command.sh"
    "${command[@]}" > "$result/build.log" 2>&1
    readelf -Ws "$result/regex_syntax_tests" > "$result/executable-symbols.txt"
    if [ "$arm" = kfrc ]; then
        grep -q 'FILE.*kfrc' "$result/executable-symbols.txt"
        # Keep all compiler temporary ELF objects for direct writer evidence.
        find "$result" -maxdepth 1 -name '*.o' -print > "$result/object-files.txt"
        test -s "$result/object-files.txt"
    fi
    sha256sum "$result/regex_syntax_tests" > "$result/outputs.sha256"
    "$result/regex_syntax_tests" --list > "$result/tests.list" 2> "$result/list.stderr"
    "$result/regex_syntax_tests" --test-threads=1 --include-ignored -Zunstable-options --format=json > "$result/tests.jsonl" 2> "$result/tests.stderr"
done
# Verify that compilation did not mutate any of the original crate sources.
python3 "$here/regex-test-config.py" "$work" > "$work/test-features.txt"
if [ "$phase" != baseline ]; then python3 "$here/regex-tests-summary.py" "$work"; fi
