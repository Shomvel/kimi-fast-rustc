#!/usr/bin/env bash
# Usage: w01.sh <workdir>   — compiles w01.rs (next to this script) and prints MIR/IR/asm counts.
# Env: RUSTC (rustc 1.94.1, host), RUSTC_X86 (a rustc with x86_64-unknown-linux-musl std).
set -u
here=$(cd "$(dirname "$0")" && pwd); out=${1:?workdir}; mkdir -p "$out"
RUSTC=${RUSTC:-rustc}; RUSTC_X86=${RUSTC_X86:-rustc}
export TMPDIR="$out"
f="-Copt-level=3 -Ccodegen-units=1 -Csymbol-mangling-version=v0"
"$RUSTC" $f -o "$out/w01" "$here/w01.rs"
"$RUSTC" $f --emit=mir -o "$out/w01.mir" "$here/w01.rs"
"$RUSTC" $f -Cno-prepopulate-passes --emit=llvm-ir -o "$out/w01.pre.ll" "$here/w01.rs"
"$RUSTC" $f --emit=llvm-ir -o "$out/w01.o3.ll" "$here/w01.rs"
"$RUSTC" $f -Cllvm-args=-print-changed=quiet --emit=obj -o "$out/w01.pc.o" "$here/w01.rs" 2> "$out/w01.changed.txt"
"$RUSTC_X86" $f --target x86_64-unknown-linux-musl --emit=asm -o "$out/w01.x86.s" "$here/w01.rs"
"$RUSTC" -vV | sed -n 's/^release: /ir rustc release: /p'
"$RUSTC_X86" -vV | sed -n 's/^release: /x86 rustc release: /p'
echo "run (size Result<u32,FieldError>, size Result<Config,Error>, ?-result ok, match-result ok): $("$out/w01")"

short() { sed -E 's/.*3w01[0-9]{1,2}(parse_config_match|parse_config)$/\1/; s/.*6Reader10next_value$/next_value/; s/.*6Reader10next_value0.*/next_value::{closure}/'; }
pat='3w01(12parse_config|18parse_config_match)$|6Reader10next_value'

echo "== MIR: 40-byte locals per function (by type)"
for fn in parse_config parse_config_match; do
  awk -v fn="$fn" '$0 ~ "^fn " fn "\\(" {p=1} p && /^}/ {p=0} p' "$out/w01.mir" |
    grep -E '^ *let ' | grep -oE 'Result<u32, FieldError>|Result<std::convert::Infallible, FieldError>|ControlFlow<[^;]*|FieldError;|Error;|Result<Config, Error>' |
    sed 's/;$//; s/std::ops:://; s/std::convert:://g; s/std::result:://g' | sort | uniq -c | sed "s/^/  $fn: /"
done
echo "== MIR: first error block of parse_config (Storage lines removed)"
awk '/^fn parse_config\(/ {p=1} p && /^}/ {p=0} p' "$out/w01.mir" |
  awk '/^    bb[0-9]+: \{/ {blk=$0; buf=""; next} /^    \}/ { if (buf ~ /Error::Field/ && !done) { print blk; printf "%s", buf; done=1 } next } { if ($0 !~ /Storage/) buf = buf $0 "\n" }' | sed 's/^/  /'
echo "== MIR: first error block of parse_config_match"
awk '/^fn parse_config_match\(/ {p=1} p && /^}/ {p=0} p' "$out/w01.mir" |
  awk '/^    bb[0-9]+: \{/ {blk=$0; buf=""; next} /^    \}/ { if (buf ~ /Error::Field/ && !done) { print blk; printf "%s", buf; done=1 } next } { if ($0 !~ /Storage/) buf = buf $0 "\n" }' | sed 's/^/  /'

for k in pre o3; do
  echo "== $k IR"
  "$here/count-ir.sh" "$out/w01.$k.ll" "$pat" | while read -r line; do
    case "$line" in TOTAL*) ;; *) sym=${line##* }; echo "${line% *} $(echo "$sym" | short)";; esac; done
  for fn in 12parse_config 18parse_config_match; do
    awk -v fn="$fn" '/^define / { f = (index($0, "3w01" fn "(") > 0) } f && /^}/ { f = 0 } f' "$out/w01.$k.ll" |
      { grep -oE 'alloca \[[0-9]+ x i8\]|llvm\.memcpy.*i64 [0-9]+, i1' || true; } |
      sed -E 's/alloca \[([0-9]+) x i8\]/alloca \1B/; s/.*i64 ([0-9]+), i1/memcpy \1B/' | sort | uniq -c | tr '\n' ' ' |
      sed "s/^/  $fn sizes: /"; echo
  done
done

echo "== LLVM IR passes that change a function's alloca/memcpy count (start = unoptimized IR)"
awk '
  FNR == 1 { file++ }
  /^\*\*\* IR Dump After / { pass = $5; next }
  /^define / { inf = 1; match($0, /@[^(]*\(/); name = substr($0, RSTART + 1, RLENGTH - 2); a = 0; m = 0; next }
  inf && / = alloca / { a++ }
  inf && /@llvm\.memcpy/ { m++ }
  inf && /^}/ { inf = 0; s = "a" a "m" m
    if (file == 1) { cur[name] = s; seq[name] = s; next }
    if (pass ~ /Pass$/ && name in cur && cur[name] != s) { seq[name] = seq[name] " >" pass "> " s; cur[name] = s } }
  END { for (k in seq) print k "|" seq[k] }' "$out/w01.pre.ll" "$out/w01.changed.txt" | grep -E '3w01(12parse_config|18parse_config_match)\|' |
while IFS='|' read -r fn seq; do echo "  $(echo "$fn" | short): $seq"; done

echo "== x86 asm (rustc x86)"
for s in 12parse_config 18parse_config_match; do
  awk -v pat="3w01$s" '
    /^[A-Za-z_.$][A-Za-z0-9_.$]*:/ && $0 !~ /^\.L/ { f = ($0 ~ pat ":$"); next }
    f && /^\.Lfunc_end/ { f = 0 }
    f && /^\t[a-z]/ { n++; if (/^\tmovups/) v++; if (/^\tcall/) c++; if (/^\tsubq\t\$[0-9]+, %rsp/) { x = $2; sub(/,$/, "", x); sub(/\$/, "", x); fr += x } }
    END { printf "  %s: inst=%d movups=%d call=%d frame=%d\n", pat, n, v, c, fr }' "$out/w01.x86.s"
done
echo "  niche constants compared/stored in parse_config (x86):"
awk '/3w0112parse_config:$/ {f=1; next} f && /^\.Lfunc_end/ {f=0} f && /\$-[0-9]+/' "$out/w01.x86.s" | sort | uniq -c | sed 's/^/   /'
echo "  niche constants in parse_config (O3 IR, 1.94.1):"
awk '/^define .*3w0112parse_config\(/ {f=1} f && /^}/ {f=0} f && /-9223372036854775[0-9]+/' "$out/w01.o3.ll" | grep -oE 'icmp eq i64 %[0-9]+, -?[0-9]+|store i64 -?[0-9]+' | sed 's/%[0-9]*//' | sort | uniq -c | sed 's/^/   /'
echo "== O3 IR: lifetime markers per 40-byte alloca (input to stack coloring)"
for fn in 12parse_config 18parse_config_match; do
  awk -v fn="$fn" '/^define / { f = (index($0, "3w01" fn "(") > 0) } f && /^}/ { f = 0 } f' "$out/w01.o3.ll" |
    { grep -oE 'lifetime\.(start|end)\.p0\(i64 40, ptr nonnull %[A-Za-z0-9_.]+' || true; } |
    sed -E 's/lifetime\.(start|end)\.p0\(i64 40, ptr nonnull (%.*)/\2 \1/' | sort | uniq -c | awk '{printf "%s %s=%s  ", $2, $3, $1} END {print ""}' | sed "s/^/  $fn: /"
done
echo "== MIR per pass (RUSTC_BOOTSTRAP=1 -Zdump-mir): discriminant reads in parse_config, printed when the count changes"
md="$out/mir-dump"; mkdir -p "$md"; rm -f "$md"/w01.parse_config.*
RUSTC_BOOTSTRAP=1 "$RUSTC" $f -Zdump-mir=parse_config -Zdump-mir-dir="$md" --emit=mir -o "$out/w01.dump.mir" "$here/w01.rs"
prev=""
for g in $(ls "$md" | grep '^w01\.parse_config\.3-2-.*\.after\.mir$' | sort); do
  c=$(grep -c 'discriminant(' "$md/$g"); [ "$c" != "$prev" ] && echo "  $c  ${g#w01.parse_config.}"; prev=$c
done
echo "  after GVN: the first Break construction and the block it jumps to"
gvn=$(ls "$md" | grep '^w01\.parse_config\.3-2-.*GVN\.after\.mir$')
awk '/^    bb[0-9]+: \{/ {b=$1; buf=""} {buf = buf $0 "\n"} /^    \}/ && buf ~ /Break\(move/ && !d {printf "%s", buf; d=1; t=buf; sub(/.*goto -> /, "", t); sub(/;.*/, "", t); print t > "/dev/stderr"}' "$md/$gvn" 2> "$out/w01.gvn-target" | grep -v Storage | sed 's/^/  /'
tgt=$(cat "$out/w01.gvn-target")
awk -v b="    $tgt: {" '$0 == b {p=1} p {print} p && /^    \}/ {exit}' "$md/$gvn" | grep -v Storage | sed 's/^/  /'
echo "== niche constants in parse_config, RUSTC_X86 compiler on the host target (O3 IR): version vs target"
"$RUSTC_X86" $f --emit=llvm-ir -o "$out/w01.x86rustc-host.o3.ll" "$here/w01.rs"
awk '/^define .*3w0112parse_config\(/ {f=1} f && /^}/ {f=0} f' "$out/w01.x86rustc-host.o3.ll" | grep -oE 'icmp eq i64 %[0-9]+, -?[0-9]+|store i64 -?[0-9]+' | sed 's/%[0-9]*//' | sort | uniq -c | sed 's/^/   /'
