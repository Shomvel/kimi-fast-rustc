#!/usr/bin/env bash
# Usage: w09.sh <workdir>   — compiles w09.rs (next to this script) and prints MIR/IR/asm counts.
# Env: RUSTC (rustc 1.94.1, host), RUSTC_X86 (a rustc with x86_64-unknown-linux-musl std).
set -u
here=$(cd "$(dirname "$0")" && pwd); out=${1:?workdir}; mkdir -p "$out"
RUSTC=${RUSTC:-rustc}; RUSTC_X86=${RUSTC_X86:-rustc}
export TMPDIR="$out"
f="-Copt-level=3 -Ccodegen-units=1 -Csymbol-mangling-version=v0"
"$RUSTC" $f -o "$out/w09" "$here/w09.rs"
"$RUSTC" $f --emit=mir -o "$out/w09.mir" "$here/w09.rs"
"$RUSTC" $f -Cno-prepopulate-passes --emit=llvm-ir -o "$out/w09.pre.ll" "$here/w09.rs"
"$RUSTC" $f --emit=llvm-ir -o "$out/w09.o3.ll" "$here/w09.rs"
"$RUSTC" $f -Cllvm-args=-print-changed=quiet --emit=obj -o "$out/w09.pc.o" "$here/w09.rs" 2> "$out/w09.changed.txt"
"$RUSTC_X86" $f --target x86_64-unknown-linux-musl --emit=asm -o "$out/w09.x86.s" "$here/w09.rs"
"$RUSTC" -vV | sed -n 's/^release: /ir rustc release: /p'
"$RUSTC_X86" -vV | sed -n 's/^release: /x86 rustc release: /p'
echo "run: $("$out/w09" | tr '\n' ' ')"

# v0 symbol -> readable name (const generic N printed in decimal)
short() {
  sed -E 's/_RNvCs[0-9A-Za-z]+_3w09[0-9]{1,2}([a-z_]+)$/\1/;
          s/.*3BoxNtCs[0-9A-Za-z]+_3w093BigE3new.*/Box<Big>::new/;
          s/.*3BoxAhj([0-9a-f]+)_E3new.*/Box<[u8;0x\1]>::new/;
          s/_RINvCs[0-9A-Za-z]+_3w09[0-9]{1,2}([a-z_]+)Kj([0-9a-f]+)_EB2_/\1<0x\2>/'
}
named='3w09[0-9]+(make_big|ret_big|box_call|box_lit|box_zero|handle|route)$|3BoxNtCs[0-9A-Za-z]+_3w093BigE3new|3BoxAhj1000_E3new'
for k in pre o3; do
  echo "== $k IR: named functions"
  "$here/count-ir.sh" "$out/w09.$k.ll" "$named" | grep -v TOTAL | while read -r line; do
    sym=${line##* }; echo "${line% *} $(echo "$sym" | short)"; done
done

echo "== MIR (Storage/debug lines removed)"
for fn in box_lit box_zero handle route; do
  awk -v fn="$fn" '$0 ~ "^fn " fn "\\(" {p=1} p && /^}/ {p=0} p' "$out/w09.mir" |
    grep -E '= |switchInt|^fn ' | grep -vE 'debug|Storage|^ *let ' | sed 's/^/  /'
done
awk '/^fn pipe\(/ {p=1} p && /^}/ {p=0} p' "$out/w09.mir" | grep -E 'take|Some|make' | sed 's/^/  pipe: /'

echo "== size thresholds: [u8; N] (pre = unoptimized IR, o3 = O3 IR; a = alloca, m = memcpy)"
tot() { "$here/count-ir.sh" "$1" "$2" | tail -1 | sed -E 's/.* alloca=([0-9]+) memcpy=([0-9]+) .*/a\1 m\2/'; }
asm() { # file symbol-substring -> "inst N memcpy-call N rep N vmov N"
  awk -v pat="$2" '
    /^[A-Za-z_.$][A-Za-z0-9_.$]*:/ && $0 !~ /^\.L/ { f = (index($0, pat) > 0); next }
    f && /^\.Lfunc_end/ { f = 0 }
    f && /^\t[a-z]/ { n++; if (/memcpy/) c++; if (/memset/) z++; if (/^\trep/) r++; if (/^\tmov[au]p[sd]/) v++; if (/^\tsubq\t\$[0-9]+, %rsp/) { x = $2; sub(/,$/, "", x); sub(/\$/, "", x); fr += x } }
    END { printf "inst=%d memcpy=%d memset=%d rep=%d vmov=%d frame=%d", n, c, z, r, v, fr }' "$1"
}
printf '%-5s | %-10s %-10s | %-10s %-10s | %-50s | %s\n' N pipe.pre pipe.o3 boxed.pre boxed.o3 "x86 pipe" "x86 boxed"
for n in 4 8 16 24 32 64 128 256 4096; do
  h=$(printf %x "$n")
  pp=$(tot "$out/w09.pre.ll" "3w094pipeKj${h}_E"); po=$(tot "$out/w09.o3.ll" "3w094pipeKj${h}_E")
  bp=$(tot "$out/w09.pre.ll" "3w095boxedKj${h}_E|3BoxAhj${h}_E3new"); bo=$(tot "$out/w09.o3.ll" "3w095boxedKj${h}_E")
  printf '%-5s | %-10s %-10s | %-10s %-10s | %-50s | %s\n' "$n" "$pp" "$po" "$bp" "$bo" \
    "$(asm "$out/w09.x86.s" "4pipeKj${h}_E")" "$(asm "$out/w09.x86.s" "5boxedKj${h}_E")"
done
echo "== pipe/boxed signature in unoptimized IR (PassMode per N)"
for n in 4 8 16 24; do h=$(printf %x "$n")
  grep -E "^define .*(4makeKj|4takeKj|8take_optKj)${h}_E" "$out/w09.pre.ll" |
    sed -E 's/ unnamed_addr.*//; s/define internal //; s/@_RINvCs[0-9A-Za-z]+_3w09[0-9]([a-z_]+)Kj[0-9a-f]+_EB2_/\1/' | sed "s/^/  N=$n: /"
done

echo "== x86 asm (named)"
for s in 8make_big 7ret_big 8box_call 7box_lit 8box_zero 6handle 5route; do
  echo "  $s: $(asm "$out/w09.x86.s" "$s")"; done

echo "== LLVM IR passes that change a function's alloca/memcpy count (start = unoptimized IR)"
# Reads the unoptimized IR first, then the -print-changed dumps; counts are attributed per `define`.
awk '
  FNR == 1 { file++ }
  /^\*\*\* IR Dump After / { pass = $5; next }
  /^define / { inf = 1; match($0, /@[^(]*\(/); name = substr($0, RSTART + 1, RLENGTH - 2); a = 0; m = 0; next }
  inf && / = alloca / { a++ }
  inf && /@llvm\.memcpy/ { m++ }
  inf && /^}/ { inf = 0; s = "a" a "m" m
    if (file == 1) { cur[name] = s; seq[name] = s; next }
    if (pass ~ /Pass$/ && name in cur && cur[name] != s) { seq[name] = seq[name] " >" pass "> " s; cur[name] = s } }
  END { for (k in seq) print k "|" seq[k] }' "$out/w09.pre.ll" "$out/w09.changed.txt" | sort |
while IFS='|' read -r fn seq; do
  name=$(echo "$fn" | short)
  case "$name" in box_call|box_lit|box_zero|handle|route|"pipe<0x100>"|"boxed<0x100>") echo "  $name: $seq";; esac
done
echo "== O3 IR: allocas and lifetime markers in pipe<256> (input to stack coloring)"
awk '/^define .*3w094pipeKj100_E/ {f=1} f && /^}/ {f=0} f' "$out/w09.o3.ll" |
  { grep -oE '%[A-Za-z0-9_.]+ = alloca \[[0-9]+ x i8\]|lifetime\.(start|end)\.p0\(i64 [0-9]+, ptr nonnull %[A-Za-z0-9_.]+' || true; } |
  sed -E 's/lifetime\.(start|end)\.p0\(i64 [0-9]+, ptr nonnull (%.*)/\2 lifetime.\1/' | sed 's/^/  /'
echo "== O3 IR: calls in pipe<256> (callee, pointer argument) and the Some payload address"
awk '/^define .*3w094pipeKj100_E/ {f=1} f && /^}/ {f=0} f' "$out/w09.o3.ll" |
  grep -E 'call fastcc|getelementptr' | sed -E 's/@_RINvCs[0-9A-Za-z]+_3w09[0-9]([a-z_]+)Kj[0-9a-f]+_EB2_/\1/; s/\(ptr noalias noundef [^%]*(%[A-Za-z0-9_]+)/(\1/; s/ #[0-9]+$//; s/^ */  /'
echo "== O3 IR: callees of box_zero / box_lit / box_call"
for fn in 8box_zero 7box_lit 8box_call; do
  awk -v fn="3w09$fn(" '/^define / { f = (index($0, fn) > 0) } f && /^}/ { f = 0 } f' "$out/w09.o3.ll" |
    { grep -oE '@(llvm\.mem[a-z]+|[A-Za-z0-9_]*(rust_alloc[a-z_]*|make_big))' || true; } | sed 's/.*___rustc[0-9]*//; s/.*3w09[0-9]//' | sort | uniq -c | tr '\n' ' ' | sed "s/^/  $fn: /"; echo
done
