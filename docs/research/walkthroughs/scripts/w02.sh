#!/usr/bin/env bash
# Usage: w02.sh <workdir>
# Env: RUSTC (rustc 1.94.1, host), RUSTC_X86 (a rustc with x86_64-unknown-linux-musl std).
set -u
here=$(cd "$(dirname "$0")" && pwd); out=${1:?workdir}; mkdir -p "$out"
RUSTC=${RUSTC:-rustc}; RUSTC_X86=${RUSTC_X86:-rustc}
pat='sum_scaled|Filter|Map|map_fold|filter_fold|Sum|slice..iter|fold'
for oc in off on; do
  f="-Copt-level=3 -Ccodegen-units=1 -Coverflow-checks=$oc"
  "$RUSTC" $f --emit=mir -o "$out/w02-$oc.mir" "$here/w02.rs"
  "$RUSTC" $f -Cno-prepopulate-passes --emit=llvm-ir -o "$out/w02-$oc.pre.ll" "$here/w02.rs"
  "$RUSTC" $f --emit=llvm-ir -o "$out/w02-$oc.o3.ll" "$here/w02.rs"
  "$RUSTC_X86" $f --target x86_64-unknown-linux-musl --emit=asm -o "$out/w02-$oc.x86.s" "$here/w02.rs"
  echo "== overflow-checks=$oc  unoptimized IR (after MIR opts), sum_scaled call tree"
  "$here/count-ir.sh" "$out/w02-$oc.pre.ll" "$pat" | sed 's/_ZN[0-9a-zA-Z_$.]*E/<sym>/' | awk '{print}' | tail -1
  "$here/count-ir.sh" "$out/w02-$oc.pre.ll" "$pat" | grep -v TOTAL | wc -l | sed 's/^ */functions listed: /'
  echo "== overflow-checks=$oc  O3 IR"
  "$here/count-ir.sh" "$out/w02-$oc.o3.ll" "$pat" | tail -1
  echo "== overflow-checks=$oc  x86 asm (rustc 1.99.1)"
  "$here/count-asm.sh" "$out/w02-$oc.x86.s" sum_scaled
done
# LLVM IR passes that changed sum_scaled (overflow-checks=off), via -print-changed on the host target.
sym=$(grep -o '_ZN3w0210sum_scaled17h[0-9a-f]*E' "$out/w02-off.o3.ll" | head -1)
"$RUSTC" -Copt-level=3 -Ccodegen-units=1 -Coverflow-checks=off --emit=obj -o "$out/w02-pc.o" \
  -Cllvm-args=-print-changed=quiet -Cllvm-args=-filter-print-funcs="$sym" "$here/w02.rs" 2> "$out/w02-print-changed.txt"
echo "== IR passes that changed sum_scaled (overflow-checks=off, host target)"
grep 'IR Dump After' "$out/w02-print-changed.txt" | grep -oE 'IR Dump After [A-Za-z0-9]+Pass' | awk '{print $4}' | sort | uniq -c | sort -rn | awk '{t+=$1; printf "%s %s; ", $1, $2} END {print "\nIR pass changes total:", t}'
