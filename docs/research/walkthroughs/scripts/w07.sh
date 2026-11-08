#!/usr/bin/env bash
# Usage: w07.sh <workdir>   — compiles w07.rs (next to this script); vtables, virtual calls, devirtualization.
# Env: RUSTC (rustc 1.94.1, host), RUSTC_X86 (a rustc with x86_64-unknown-linux-musl std).
set -u
here=$(cd "$(dirname "$0")" && pwd); out=${1:?workdir}; mkdir -p "$out"
RUSTC=${RUSTC:-rustc}; RUSTC_X86=${RUSTC_X86:-rustc}
f="-Copt-level=3 -Ccodegen-units=1"
"$RUSTC" $f --emit=mir -o "$out/w07.mir" "$here/w07.rs"
"$RUSTC" $f -Cno-prepopulate-passes --emit=llvm-ir -o "$out/w07.pre.ll" "$here/w07.rs"
"$RUSTC" $f --emit=llvm-ir -o "$out/w07.o3.ll" "$here/w07.rs"
"$RUSTC_X86" $f --target x86_64-unknown-linux-musl --emit=asm -o "$out/w07.x86.s" "$here/w07.rs"
"$RUSTC_X86" -vV | sed -n 's/^release: /x86 rustc release: /p'

# Legacy-mangled symbol -> readable path without hash.
demangle() {
  awk '{ s=$0; out=""; if (substr(s,1,3)=="_ZN") { s=substr(s,4)
           while (match(s, /^[0-9]+/)) { n=substr(s,1,RLENGTH)+0; s=substr(s,RLENGTH+1); seg=substr(s,1,n); s=substr(s,n+1)
             if (seg ~ /^h[0-9a-f]{16}$/) continue; out=out (out==""?"":"::") seg }
           gsub(/\$LT\$/,"<",out); gsub(/\$GT\$/,">",out); gsub(/\$u20\$/," ",out); gsub(/\$u7b\$/,"{",out); gsub(/\$u7d\$/,"}",out)
           gsub(/\.\./,"::",out); gsub(/\$C\$/,",",out); gsub(/^_</,"<",out); print out }
         else print s }'
}
# @vtable.N globals: decode byte-array words (little-endian u64) and name the function slots.
vtables() { # file
  grep '^@vtable' "$1" | awk '
    BEGIN { for (i=32; i<127; i++) ord[sprintf("%c", i)]=i; hx="0123456789ABCDEF" }
    function h2(s) { return (index(hx, toupper(substr(s,1,1)))-1)*16 + index(hx, toupper(substr(s,2,1)))-1 }
    function words(s,   i, c, b, nb, w, k, r) { nb=0; i=1
      while (i<=length(s)) { c=substr(s,i,1); if (c=="\\") { b[nb++]=h2(substr(s,i+1,2)); i+=3 } else { b[nb++]=ord[c]; i++ } }
      r=""; for (k=0; k+7<nb; k+=8) { w=0; for (i=7; i>=0; i--) w=w*256+b[k+i]; r=r " " w } return r }
    { name=$1; body=$0; sub(/^[^<]*<\{[^}]*\}> <\{ /, "", body); sub(/ \}>, align.*/, "", body)
      line=name ":"; n=split(body, it, /, (ptr|\[)/)
      while (match(body, /(ptr @"?[^", ]+"?|\[[0-9]+ x i8\] c"[^"]*")/)) { t=substr(body, RSTART, RLENGTH); body=substr(body, RSTART+RLENGTH)
        if (t ~ /^ptr/) { s=t; sub(/^ptr @"?/, "", s); sub(/"$/, "", s); line=line " | fn " s }
        else { s=t; sub(/^[^"]*c"/, "", s); sub(/"$/, "", s); line=line " | words" words(s) } }
      print line }' | while IFS= read -r l; do
        out=""; for tok in $(echo "$l" | tr ' ' '\n'); do case "$tok" in _ZN*) out="$out $(echo "$tok" | demangle)";; *) out="$out $tok";; esac; done; echo " $out"; done
}
# Per-function direct/indirect calls (instructions only; rustc comment lines skipped).
calls() { # file fn-regex
  awk -v pat="$2" '/^define / { if (match($0, /@"?[^"( ]+"?\(/)) { nm=substr($0, RSTART+1, RLENGTH-2); gsub(/"/, "", nm) } f=($0 ~ pat); n=0; d=0; i=0; v=0; lp=0; next }
    f && /^}/ { printf "  inst=%-4d direct=%-3d indirect=%-2d invoke=%-2d landingpad=%-2d %s\n", n, d, i, v, lp, nm; f=0 }
    f && /^  [^ ;]/ { n++; if (/ invoke /) v++; if (/ landingpad /) lp++
      if ((/(^| )call / || / invoke /) && !/@llvm\./) { if (/(call|invoke) [^@%]*%[A-Za-z0-9_.]+\(/) i++; else d++ } }' "$1" \
  | while IFS= read -r l; do sym=${l##* }; echo "${l% *} $(echo "$sym" | demangle)"; done
}
pat='4dynk|drop_in_place.*Box.*dyn|Rect.*as.*Shape|Sq.*as.*Shape|5Shape4name'
echo "== MIR: unsize coercions and calls in dynk fns"
awk '/^fn (total|local_devirt|emit)\(/ {fn=$2; sub(/\(.*/, "", fn)} /^}/ {fn=""} fn != "" && /Unsize/ {u[fn]++} fn != "" && / = .*\(.*\) -> / {c[fn]++; if (/<dyn /) dv[fn]++}
     END { for (k in u) printf "  unsize=%d %s\n", u[k], k; for (k in c) printf "  call_terminators=%d virtual=%d %s\n", c[k], dv[k], k }' "$out/w07.mir" | sort
echo "== vtables in pre IR (CGU=1)"; vtables "$out/w07.pre.ll"
echo "== function-pointer entries per vtable (incl. drop slot when non-null)"
grep "^@vtable" "$out/w07.pre.ll" | awk '{ n=gsub(/ptr @/, "&"); printf "  %s fn_entries=%d\n", $1, n }'
for st in pre o3; do echo "== $st IR calls"; calls "$out/w07.$st.ll" "$pat"; done
echo "== O3 IR: vtables left and their use count"
for v in $(grep -oE '^@vtable\.[0-9]+' "$out/w07.o3.ll"); do echo "  $v uses=$(grep -c "$v[^0-9]" "$out/w07.o3.ll" | awk '{print $1-1}')"; done
echo "== vtable copies per CGU (pre IR, -Ccodegen-units=16)"
mkdir -p "$out/cgu16"; rm -f "$out"/cgu16/*.ll
"$RUSTC" -Copt-level=3 -Ccodegen-units=16 -Cno-prepopulate-passes --emit=llvm-ir --out-dir "$out/cgu16" "$here/w07.rs"
for g in "$out"/cgu16/*.ll; do
  nv=$(grep -c '^@vtable' "$g"); [ "$nv" = 0 ] && continue
  c=${g##*-cgu.}; echo "  cgu.${c%%.*}: vtables=$nv rect_shape_vtables=$(grep '^@vtable' "$g" | grep -c 'drop_in_place$LT$w07..Rect') fns: $(grep '^define' "$g" | grep -oE '3w07(4dynk)?[0-9]+[a-z_]+17h' | sed -E 's/^3w07(4dynk)?[0-9]+//;s/17h$//' | sort -u | tr '\n' ' ')"
done
echo "== x86 asm (rustc 1.99.1)"
for s in 12local_devirt 5total 4emit; do "$here/count-asm.sh" "$out/w07.x86.s" "4dynk$s"; done
echo "   indirect calls via register/memory, GOT excluded: $(awk '/4dynk[0-9a-z_]*:$/ {f=1} f && /^\.Lfunc_end/ {f=0} f && /\tcallq?\t\*/ && !/GOTPCREL/ {n++} END {print n+0}' "$out/w07.x86.s")"
echo "== pre IR: parameter attributes of dynk::emit (&mut dyn Write = data ptr + vtable ptr)"
grep -E '^define .*4dynk4emit' "$out/w07.pre.ll" | sed -E 's/^[^(]*\(//; s/\) unnamed_addr.*//; s/, i64 noundef %0$//' | tr -s ' ' | sed 's/, ptr /\n  ptr /; s/^/  /'
echo "== pre IR: pointer loads with !invariant.load / all pointer loads, in dynk fns"
awk '/^define .*4dynk/ {f=1} /^}/ {f=0} f && / = load ptr, ptr %[0-9]+, align 8/ {n++; if (/invariant\.load/) i++} END {print "  " i+0 " / " n+0}' "$out/w07.pre.ll"
echo "== pre IR: drop_in_place<Box<dyn _>> shims: distinct bodies, raw and with callee symbols masked"
awk '/^define .*drop_in_place.*Box.*dyn/ {f=1; b=""; next} f && /^}/ {f=0; h[b]++; hm[m]++; m=""; n++; next} f && /^  [^ ;]/ {l=$0; sub(/, !dbg.*/, "", l); b=b l "\n"; gsub(/@"?[^"( ]+"?/, "@SYM", l); m=m l "\n"}
     END { u=0; for (k in h) u++; um=0; for (k in hm) um++; print "  shims=" n " distinct_bodies=" u " distinct_with_symbols_masked=" um }' "$out/w07.pre.ll"
echo "== pre IR: allocator attributes on the __rust_alloc declaration"
g=$(grep -E '^declare .*___rust_alloc\(' "$out/w07.pre.ll" | grep -oE '#[0-9]+$')
echo "  $(grep -E "^attributes $g = " "$out/w07.pre.ll" | grep -oE 'nounwind|allockind\("[^"]*"\)|allocsize\([0-9]+\)' | tr '\n' ' ')"
