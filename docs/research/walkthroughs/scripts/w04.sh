#!/usr/bin/env bash
# Usage: w04.sh <workdir>   — compiles w04.rs (next to this script) under panic=unwind and panic=abort.
# Env: RUSTC (rustc 1.94.1, host), RUSTC_X86 (a rustc with x86_64-unknown-linux-musl std).
set -u
here=$(cd "$(dirname "$0")" && pwd); out=${1:?workdir}; mkdir -p "$out"
RUSTC=${RUSTC:-rustc}; RUSTC_X86=${RUSTC_X86:-rustc}
"$RUSTC_X86" -vV | sed -n 's/^release: /x86 rustc release: /p'

# Legacy-mangled symbol -> path without hash (w04::may_panic).
demangle() {
  awk '{ s=$0; out=""; if (substr(s,1,3)=="_ZN") { s=substr(s,4)
           while (match(s, /^[0-9]+/)) { n=substr(s,1,RLENGTH)+0; s=substr(s,RLENGTH+1); seg=substr(s,1,n); s=substr(s,n+1)
             if (seg ~ /^h[0-9a-f]{16}$/) continue; out=out (out==""?"":"::") seg }
           gsub(/\$LT\$/,"<",out); gsub(/\$GT\$/,">",out); gsub(/\$u20\$/," ",out); gsub(/\.\./,"::",out); gsub(/\$C\$/,",",out); print out }
         else print s }'
}
# MIR terminator/flag counts for one fn.
mir_counts() { # file fn-name
  awk -v fn="$2" '
    $0 ~ "^fn " fn "\\(" { f=1; next } f && /^}/ { f=0 }
    f && /^    bb[0-9]+/ { b++; if (/\(cleanup\)/) cb++ }
    f && /drop\(_[0-9]+\) ->/ { d++ }
    f && / = [^ ]*\(.*\) -> / { c++ }
    f && /unwind: bb/ { ub++ }  f && /unwind continue/ { uc++ }  f && /unwind unreachable/ { uu++ }
    f && /unwind terminate/ { ut++ }  f && /^        resume;/ { r++ }
    f && /_[0-9]+ = const (true|false);/ { fl++ }
    END { printf "blocks=%d cleanup_blocks=%d drop=%d calls=%d unwind_to_bb=%d unwind_continue=%d unwind_unreachable=%d unwind_terminate=%d resume=%d drop_flag_writes=%d\n", b, cb, d, c, ub, uc, uu, ut, r, fl }' "$1"
}
# Call sites inside one IR function: "call|invoke  callee".
call_sites() { # file fn-regex
  awk -v pat="$2" '/^define / { f=($0 ~ pat); next } f && /^}/ { f=0 }
    f && /^  [^ ;]/ && /(^| )(call|invoke) / && !/@llvm\./ { k=($0 ~ /invoke /)?"invoke":"call"
      if (match($0, /@"?[^"( ]+"?\(/)) { s=substr($0, RSTART+1, RLENGTH-2); gsub(/"/, "", s); print k, s } }' "$1" \
  | while read -r k s; do printf '  %-6s %s\n' "$k" "$(echo "$s" | demangle)"; done | sort | uniq -c
}

# count-ir.sh also counts the "; call X" / "; invoke X" comment lines rustc emits; recount on instructions only.
real_counts() { # file fn-regex
  awk -v pat="$2" '/^define / { if ($0 ~ pat) { f=1; F++ } next } f && /^}/ { f=0 }
    f && /^  [^ ;]/ { if (/ invoke / || /^  invoke /) v++; else if (/(^| )call / && !/@llvm\./) c++
                      if (/ landingpad /) lp++; if (/^  resume /) r++; if (/ = phi i1 /) ph++; if (/ = select i1 /) se++; if (/ = phi { ptr, i32 }/) pe++ }
    END { printf "functions=%d call=%d invoke=%d landingpad=%d resume=%d phi_i1=%d select_i1=%d phi_exc=%d\n", F, c, v, lp, r, ph, se, pe }' "$1"
}

for p in unwind abort; do
  f="-Copt-level=3 -Ccodegen-units=1 -Cpanic=$p"
  "$RUSTC" $f --emit=mir -o "$out/w04-$p.mir" "$here/w04.rs"
  "$RUSTC" $f -Cno-prepopulate-passes --emit=llvm-ir -o "$out/w04-$p.pre.ll" "$here/w04.rs"
  "$RUSTC" $f --emit=llvm-ir -o "$out/w04-$p.o3.ll" "$here/w04.rs"
  "$RUSTC_X86" $f --target x86_64-unknown-linux-musl --emit=asm -o "$out/w04-$p.x86.s" "$here/w04.rs"
  echo "==== panic=$p"
  echo "[$p] MIR work: $(mir_counts "$out/w04-$p.mir" work)"
  for st in pre o3; do
    echo "[$p $st] count-ir work: $("$here/count-ir.sh" "$out/w04-$p.$st.ll" '3w044work' | tail -1 | sed 's/^TOTAL //')"
    echo "[$p $st] work, instructions only: $(real_counts "$out/w04-$p.$st.ll" '3w044work')"
    echo "[$p $st] whole module: $(real_counts "$out/w04-$p.$st.ll" '.')"
    echo "[$p $st] drop_in_place fns: $(real_counts "$out/w04-$p.$st.ll" 'drop_in_place')"
    echo "[$p $st] call sites in work:"; call_sites "$out/w04-$p.$st.ll" '3w044work'
  done
  line="[$p o3] nounwind attribute:"
  for fnm in may_panic no_panic even odd consume; do
    g=$(grep -E "^define .*3w04[0-9]+${fnm}17h" "$out/w04-$p.o3.ll" | grep -oE '#[0-9]+' | tail -1)
    if [ -z "$g" ]; then v=inlined; elif grep -E "^attributes $g = " "$out/w04-$p.o3.ll" | grep -q nounwind; then v=yes; else v=no; fi
    line="$line $fnm=$v"
  done
  echo "$line"
  echo "[$p x86] work: $("$here/count-asm.sh" "$out/w04-$p.x86.s" 3w044work | awk '{print $1, $2}') _Unwind_Resume_refs=$(awk '/3w044work.*:$/ {f=1} f && /^\.Lfunc_end/ {f=0} f && /_Unwind_Resume/ {n++} END {print n+0}' "$out/w04-$p.x86.s")"
done

# Bottom-up summary schedule on the pre-LLVM call graph (panic=unwind): height = 1 + max callee height,
# leaves = 0; a caller can only finish S-level after all callees. Back edges (cycles) are ignored and counted.
echo "==== call graph of defined functions (pre IR, panic=unwind)"
awk '
  /^define / { if (match($0, /@"?[^"( ]+"?\(/)) { cur=substr($0, RSTART+1, RLENGTH-2); gsub(/"/, "", cur); def[cur]=1; nd++ } next }
  /^}/ { cur=""; next }
  cur != "" && /^  [^ ;]/ && /(^| )(call|invoke) / && match($0, /@"?[^"( ]+"?\(/) {
    s=substr($0, RSTART+1, RLENGTH-2); gsub(/"/, "", s); if (!((cur SUBSEP s) in seen)) { seen[cur SUBSEP s]=1; e[cur]=e[cur] " " s } }
  function h(f,   n, a, i, c, m, x) {
    if (f in H) return H[f]; if (f in on) { back++; return -1 }
    on[f]=1; m=0; n=split(e[f], a, " ")
    for (i=1; i<=n; i++) { c=a[i]; if (!(c in def)) continue; edges++; x=h(c); if (x+1>m) m=x+1 }
    delete on[f]; H[f]=m; return m }
  END { for (f in def) { x=h(f); w[x]++; if (x>mx) mx=x }
        printf "defined=%d local_edges=%d back_edges=%d max_height=%d\n", nd, edges, back, mx
        printf "functions per height:"; for (i=0; i<=mx; i++) printf " %d:%d", i, w[i]; printf "\n"
        printf "height of work: %d\n", H[wk] }
' wk="$(grep -oE '^define [^@]*@_ZN3w044work17h[0-9a-f]+E' "$out/w04-unwind.pre.ll" | sed 's/.*@//')" "$out/w04-unwind.pre.ll"
