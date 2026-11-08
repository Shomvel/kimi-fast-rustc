#!/usr/bin/env bash
# Usage: w03.sh <workdir>   — compiles w03.rs (next to this script), runs it, prints MIR/IR/asm facts.
# Env: RUSTC (rustc 1.94.1, host), RUSTC_X86 (a rustc with x86_64-unknown-linux-musl std).
set -u
here=$(cd "$(dirname "$0")" && pwd); out=${1:?workdir}; mkdir -p "$out"
RUSTC=${RUSTC:-rustc}; RUSTC_X86=${RUSTC_X86:-rustc}
f="-Copt-level=3 -Ccodegen-units=1"
"$RUSTC" $f -o "$out/w03" "$here/w03.rs" && "$out/w03" && echo "run: ok"
"$RUSTC" $f --emit=mir -o "$out/w03.mir" "$here/w03.rs"
"$RUSTC" $f -Cno-prepopulate-passes --emit=llvm-ir -o "$out/w03.pre.ll" "$here/w03.rs"
"$RUSTC" $f --emit=llvm-ir -o "$out/w03.o3.ll" "$here/w03.rs"
"$RUSTC_X86" $f --target x86_64-unknown-linux-musl --emit=asm -o "$out/w03.x86.s" "$here/w03.rs"
"$RUSTC_X86" -vV | sed -n 's/^release: /x86 rustc release: /p'
pat='doctestable|any_lib|kind_eq|3w034same|4find|SlicePartialEq'
short() { sed -E 's/_ZN[0-9a-zA-Z_$.]*(doctestable|any_lib|kind_eq|same|find|SlicePartialEq)[0-9a-zA-Z_$.]*closure[0-9a-zA-Z_$.]*E"?/<\1::closure>/; s/_ZN[0-9a-zA-Z_$.]*(doctestable|any_lib|kind_eq|same|find|SlicePartialEq)[0-9a-zA-Z_$.]*E"?/<\1>/; s/"//g'; }
fn_body() { awk -v p="$2" '/^define / && $0 ~ p {f=1} f {print} f && /^}/ {f=0}' "$1"; }

echo "== unoptimized IR (after MIR opts), user functions + reached callees"
"$here/count-ir.sh" "$out/w03.pre.ll" "$pat" | short
echo "== O3 IR"
"$here/count-ir.sh" "$out/w03.o3.ll" "$pat" | short
echo "== unoptimized IR: Option/niche decode in doctestable (sub / icmp u* / select)"
fn_body "$out/w03.pre.ll" doctestable | grep -E ' = sub | icmp u(ge|lt|gt|le) | = select ' | sed 's/, !dbg.*//'
echo "== unoptimized IR: icmp eq against variant constants in doctestable"
fn_body "$out/w03.pre.ll" doctestable | grep -E 'icmp eq i64 %[^,]*, [0-9]+$' | sed 's/, !dbg.*//'
echo "== unoptimized IR: SlicePartialEq::equal calls (all / with poison args)"
grep -c 'call .*SlicePartialEq.*equal' "$out/w03.pre.ll"
grep -c 'call .*SlicePartialEq.*equal.*poison' "$out/w03.pre.ll"
echo "== unoptimized IR: llvm.assume / load ptr with !nonnull / niche decodes (icmp uge 2^63), per function"
for fn in '3w0311doctestable17h' '3w037any_lib17h' 'any_lib28_' '3Map4find17h'; do
  b=$(fn_body "$out/w03.pre.ll" "$fn")
  echo "  $fn: assume=$(grep -c 'llvm.assume' <<<"$b") load-ptr-!nonnull=$(grep -c '= load ptr.*!nonnull' <<<"$b") niche-decode=$(grep -c 'icmp uge i64 .*-9223372036854775808' <<<"$b")"
done
echo "== CrateType niche: !range on the tag load (pre IR) and field offsets used for Other(String)"
grep -m1 -E '^!6 = ' "$out/w03.pre.ll"
fn_body "$out/w03.pre.ll" 'any_lib28_' | grep -m2 -E 'getelementptr inbounds i8, ptr %k, i64 (8|16)$' | sed 's/^ */  /'
echo "== MIR: discriminant reads per function (paths stripped)"
awk '/^fn / {name=$0; sub(/\(.*/, "", name); gsub(/[^ ]*\//, "", name)} /discriminant\(/ {c[name]++} END {for (k in c) printf "  %d  %s\n", c[k], k}' "$out/w03.mir" | sort -k2
echo "== O3 IR: compares/switch in doctestable and any_lib"
for fn in '3w0311doctestable17h' '3w037any_lib17h'; do
  echo "  [$fn]"; fn_body "$out/w03.o3.ll" "$fn" | grep -E 'icmp|switch|^ +i64 [0-9]|xor|and i64|add i64' | sed -E 's/, !dbg.*//; s/label %"?[^ ]*/label <exit>/g; s/^ */    /'
done
echo "== LLVM passes that changed each function (O3 pipeline, host target)"
for name in doctestable any_lib; do
  sym=$(grep -o "_ZN3w03[0-9]*${name}17h[0-9a-f]*E" "$out/w03.pre.ll" | head -1)
  "$RUSTC" $f --emit=llvm-ir -o "$out/w03.changed.ll" -Cllvm-args=-print-changed=quiet -Cllvm-args=-filter-print-funcs="$sym" "$here/w03.rs" 2>&1 \
    | grep -o 'IR Dump After [A-Za-z]*' | sed 's/IR Dump After //' | sort | uniq -c | sort -rn \
    | awk -v n="$name" '{s+=$1; l=l sprintf("%d %s; ", $1, $2)} END {printf "  %s: total %d: %s\n", n, s, l}'
done
echo "== x86 asm (rustc release printed above)"
for s in doctestable any_lib kind_eq 3w034same; do "$here/count-asm.sh" "$out/w03.x86.s" "$s"; done
echo "== CrateType size: dereferenceable on same(&CrateType, ..) parameters (pre IR)"
grep -m1 '^define .*3w034same' "$out/w03.pre.ll" | grep -o 'dereferenceable([0-9]*) %[a-z]*' | sed 's/^/  /'
echo "== O3 IR: niche decodes (xor/slt/select) in same"
b=$(fn_body "$out/w03.o3.ll" '3w034same17h'); echo "  xor=$(grep -c ' = xor ' <<<"$b") icmp-slt=$(grep -c 'icmp slt' <<<"$b") select=$(grep -c ' = select ' <<<"$b")"
echo "== x86 asm: tag tests in doctestable / any_lib"
for s in 3w0311doctestable 3w037any_lib; do
  awk -v p="$s" 'index($0, p) && /:$/ {f=1; next} f && /^\.Lfunc_end/ {f=0} f && /\t(andq|movabsq|btq|cmpq|addq\t%rdx|sete|movl\t\$[0-9]+, %esi)/' "$out/w03.x86.s" | sed "s/^\t/  $s: /; s/\t/ /g"
done
