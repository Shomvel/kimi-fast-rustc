#!/usr/bin/env bash
# Real public CLI -> pinned rustc -> sole semantic backend, with object evidence.
set -euo pipefail
repo=$(realpath "${1:?repository}")
backend=$(realpath "${2:?backend dylib}")
out=${3:?fresh result directory}
if [ -e "$out" ]; then printf 'existing output: %s\n' "$out" >&2; exit 2; fi
mkdir -p "$out/objects"
out=$(realpath "$out")
run() {
    local name=$1
    shift
    local status=0
    printf '%q ' "$@" > "$out/$name.command"
    printf '\n' >> "$out/$name.command"
    "$@" > "$out/$name.stdout" 2> "$out/$name.stderr" || status=$?
    printf '{"exit_code":%s}\n' "$status" > "$out/$name.result.json"
    if [ "$status" != 0 ]; then cat "$out/$name.stderr" >&2; return "$status"; fi
}
run cli-build cargo +1.94.1 build --manifest-path "$repo/Cargo.toml" --bin kfrc
cli="$repo/target/debug/kfrc"
cat > "$out/library.rs" <<'RS'
#[inline(never)]
pub fn compute(x: u64) -> u64 { let mut pair = (x, 9u64); let address = &mut pair; address.0 += 3; address.0 ^ address.1 }
RS
cat > "$out/main.rs" <<'RS'
extern crate cli_probe;
fn check(x: u64) { let got=cli_probe::compute(x); assert_eq!(got,x.wrapping_add(3)^9); println!("{x}:{got}"); }
fn main() { check(0); check(7); check(u64::MAX); }
RS
clean=(env -u KFRC_PROFILE -u KFRC_PROFILE_TOTAL -u KFRC_LLVM_PROFILE -u KFRC_SPECIALIZATION_STATS -u KFRC_INLINE -u KFRC_OPTIMIZE -u KFRC_DUMP_BODY -u KFRC_INTERPRET KFRC_CODEGEN_BACKEND="$backend" KFRC_DUMP_GRAPH=1)
run cli-library "${clean[@]}" "$cli" native "$out/library.rs" --edition=2021 --crate-type=rlib --crate-name=cli_probe -Coverflow-checks=off -Cdebug-assertions=off -Ccodegen-units=1 -o "$out/libcli_probe.rlib"
run cli-executable "${clean[@]}" "$cli" native "$out/main.rs" --edition=2021 --extern=cli_probe="$out/libcli_probe.rlib" -Coverflow-checks=off -Cdebug-assertions=off -Ccodegen-units=1 -o "$out/program"
run cli-execute "$out/program"
run llvm-executable env RUSTC_BOOTSTRAP=1 rustc +1.94.1 "$out/main.rs" --edition=2021 -Cpanic=abort -Zmir-opt-level=0 --extern=cli_probe="$out/libcli_probe.rlib" -Coverflow-checks=off -Cdebug-assertions=off -o "$out/reference"
run llvm-execute "$out/reference"
cmp "$out/cli-execute.stdout" "$out/llvm-execute.stdout"
(cd "$out/objects" && ar x ../libcli_probe.rlib)
readelf -Ws "$out/objects"/*.o > "$out/object-symbols.txt"
readelf -Ws "$out/program" > "$out/executable-symbols.txt"
grep -q 'FILE.*kfrc' "$out/object-symbols.txt"
grep -q 'FILE.*kfrc' "$out/executable-symbols.txt"
grep -q 'kfrc semantic input' "$out/cli-library.stderr"
grep -q 'kfrc semantic input' "$out/cli-executable.stderr"
sha256sum "$cli" "$backend" "$out/library.rs" "$out/main.rs" "$out/libcli_probe.rlib" "$out/program" > "$out/fingerprints.sha256"
printf 'PASS: native CLI compiles actual Rust library and executable through semantic IR\n'
