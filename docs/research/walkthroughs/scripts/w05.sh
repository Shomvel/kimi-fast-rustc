#!/usr/bin/env bash
# Usage: w05.sh <workdir>   — compiles w05.rs / w05-contrast.rs (next to this script), prints MIR/IR/asm facts.
# Env: RUSTC (rustc 1.94.1, host), RUSTC_X86 (a rustc with x86_64-unknown-linux-musl std).
set -u
here=$(cd "$(dirname "$0")" && pwd); out=${1:?workdir}; mkdir -p "$out"
RUSTC=${RUSTC:-rustc}; RUSTC_X86=${RUSTC_X86:-rustc}
f="-Copt-level=3 -Ccodegen-units=1"
"$RUSTC" $f --emit=mir -o "$out/w05.mir" "$here/w05.rs"
"$RUSTC" $f -Cno-prepopulate-passes --emit=llvm-ir -o "$out/w05.pre.ll" "$here/w05.rs"
"$RUSTC" $f --emit=llvm-ir -o "$out/w05.o3.ll" "$here/w05.rs"
"$RUSTC_X86" $f --target x86_64-unknown-linux-musl --emit=asm -o "$out/w05.x86.s" "$here/w05.rs"
"$RUSTC_X86" $f --crate-type=lib --target x86_64-unknown-linux-musl --emit=asm -o "$out/w05c.x86.s" "$here/w05-contrast.rs"
"$RUSTC_X86" $f --crate-type=lib --target x86_64-unknown-linux-musl --emit=llvm-ir -o "$out/w05c.o3.ll" "$here/w05-contrast.rs"
echo "IR rustc: $("$RUSTC" -V)   x86 rustc: $("$RUSTC_X86" -V)"

echo "== MIR append: inlined scopes / remaining calls"
awk '/^fn append\(/,/^}/' "$out/w05.mir" | grep -o 'inlined [^)]*)\|= [A-Za-z:<>_]*push_mut[^;]*' | sed 's/^/  /'

pat='3w056append17|3w0510append_ext17|push_mut|Iter.*Iterator.*4next|reserve|spec_extend'
echo "== unoptimized IR (after MIR opts)"
"$here/count-ir.sh" "$out/w05.pre.ll" "$pat" | sed -E 's/ "?_ZN[^ ]*(append_ext|append|push_mut|4next|do_reserve_and_handle|7reserve|spec_extend)[^ ]*$/ \1/'

echo "== O3 IR: parameter attributes"
sed -nE 's/^define [^@]*@_ZN3w05(6append|10append_ext)17h[0-9a-f]*E\((.*)\) unnamed_addr.*/  \1: \2/p' "$out/w05.o3.ll"
sed -nE 's/^declare void @"?_ZN5alloc7raw_vec[^(]*grow_one[^(]*\((.*)\) unnamed_addr.*/  declare grow_one: \1/p' "$out/w05.o3.ll"
sed -nE 's/^define [^@]*@_R[A-Za-z0-9_]*(append_my|append_raw|4grow)\((.*)\) unnamed_addr.*/  contrast \1: \2/p' "$out/w05c.o3.ll"
echo "== O3 IR append: memory ops and calls in append"
awk '/^define .*6append17/ {f=1} f && /^}/ {f=0} f && (/ = load / || /^  store / || /^  [^;]* call /) {sub(/, !.*/, ""); sub(/^ +/, ""); print "  " $0}' "$out/w05.o3.ll"

echo "== x86 asm (one line per basic block; directives dropped)"
blocks() { # file symbol-substring
  awk -v pat="$2" '
    /^[A-Za-z_][A-Za-z0-9_.$]*:/ { if (index($0, pat) > 0) { f=1; printf "  %s\n", $0; next } }
    f && /^\.Lfunc_end/ { if (line != "") print "    " line; f=0; line=""; next }
    f && /^\.LBB/ { if (line != "") print "    " line; line=$0; next }
    f && /^\t[a-z]/ { s=$0; sub(/^\t/, "", s); gsub(/\t/, " ", s); if (s ~ /^(push|pop)/) next; line = (line == "") ? s : line " ; " s }
  ' "$1"
}
for s in 3w056append: 3w0510append_ext:; do "$here/count-asm.sh" "$out/w05.x86.s" "$s"; blocks "$out/w05.x86.s" "$s"; done
for s in append_raw: append_my:; do "$here/count-asm.sh" "$out/w05c.x86.s" "$s"; blocks "$out/w05c.x86.s" "$s"; done
