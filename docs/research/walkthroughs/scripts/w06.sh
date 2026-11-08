#!/usr/bin/env bash
# Usage: w06.sh <workdir>   — compiles w06.rs (next to this script); prints instance counts and duplicate classes.
# Env: RUSTC (rustc 1.94.1, host), RUSTC_X86 (a rustc with x86_64-unknown-linux-musl std),
#      RUSTC_NIGHTLY (optional, for -Zprint-mono-items), CXXFILT (llvm-cxxfilt; demangles v0 symbols).
# Symbols use -Csymbol-mangling-version=v0 so instances carry readable type args; this changes names only.
set -u
here=$(cd "$(dirname "$0")" && pwd); out=${1:?workdir}; mkdir -p "$out"
RUSTC=${RUSTC:-rustc}; RUSTC_X86=${RUSTC_X86:-rustc}; RUSTC_NIGHTLY=${RUSTC_NIGHTLY:-}; CXXFILT=${CXXFILT:-llvm-cxxfilt}
f="-Copt-level=3 -Ccodegen-units=1 -Csymbol-mangling-version=v0"
"$RUSTC" $f --emit=mir -o "$out/w06.mir" "$here/w06.rs"
"$RUSTC" $f -Cno-prepopulate-passes --emit=llvm-ir -o "$out/w06.pre.ll" "$here/w06.rs"
"$RUSTC" $f --emit=llvm-ir -o "$out/w06.o3.ll" "$here/w06.rs"
# Same O3 pipeline with LLVM MergeFunctions off (rustc default is Aliases), to separate it from dedup by IR shape.
RUSTC_BOOTSTRAP=1 "$RUSTC" $f -Zmerge-functions=disabled --emit=llvm-ir -o "$out/w06.o3nomerge.ll" "$here/w06.rs"
"$RUSTC_X86" $f --target x86_64-unknown-linux-musl --emit=asm -o "$out/w06.x86.s" "$here/w06.rs"
echo "rustc: $("$RUSTC" -V)   x86 rustc: $("$RUSTC_X86" -V)"

dm() { "$CXXFILT" | sed -e 's/\[[0-9a-f]*\]//g' -e 's/::h[0-9a-f]\{16\}//g' -e 's/"//g' \
  -e 's/\$LT\$/</g; s/\$GT\$/>/g; s/\$u20\$/ /g; s/\$LP\$/(/g; s/\$RP\$/)/g; s/\$C\$/,/g; s/\.\./::/g; s/_</</g'; }
generalize() { sed -E 's/\[u8; 4\]|u32|i32|char|u64|f64/T/g'; }
defs() { grep '^define ' "$1" | sed -E 's/.*@("[^"]*"|[^ (]*)\(.*/\1/'; }

echo; echo "== instances per definition (unoptimized IR defines: $(grep -c '^define ' "$out/w06.pre.ll"); O3 IR defines: $(grep -c '^define ' "$out/w06.o3.ll"); O3 without MergeFunctions: $(grep -c '^define ' "$out/w06.o3nomerge.ll"))"
pick='largest|push_one|push_mut|grow_one|PartialOrd|drop_in_place|as core::ops::drop::Drop'
for k in pre o3; do defs "$out/w06.$k.ll" | dm | generalize | sort | uniq -c | grep -E "$pick" | sed "s/^/  $k /"; done
if [ -n "$RUSTC_NIGHTLY" ]; then
  "$RUSTC_NIGHTLY" -Copt-level=3 -Ccodegen-units=1 -Zprint-mono-items=yes --emit=link -o "$out/w06n" "$here/w06.rs" > "$out/w06.monoitems" 2>&1
  echo "  nightly $("$RUSTC_NIGHTLY" -V | cut -d' ' -f2-4): MONO_ITEM count = $(grep -c '^MONO_ITEM' "$out/w06.monoitems")"
fi

echo; echo "== per-instance IR size (count-ir.sh)"
for k in pre o3; do "$here/count-ir.sh" "$out/w06.$k.ll" 'largest|push_one|push_mut|grow_one|PartialOrd' | dm | sed -E "s/^ *([0-9]+) inst .*call=([0-9]+) .*store=[0-9]+ +/  $k \1 inst call=\2  /"; done

# Duplicate classes. Exact: bodies equal after renaming self. Fixpoint: callees compared by class
# (partition refinement from "all callees equal", i.e. -Zmono-dedup's callee-class fixpoint).
# IR normalisation: drop comments; resolve #attr groups and !range/!align/!noundef/!nonnull/!prof to their
# contents (never strip them: that would hide unsound merges); renumber other metadata and all local
# names per function (inlined labels carry the callee symbol);
# non-function globals compare by their initializer.
classes() { # file kind(ir|asm)
  awk -v kind="$2" '
    function tokre() { return kind == "ir" ? "@(\"[^\"]*\"|[-A-Za-z0-9_.$]+)" : "(_R|_ZN)[A-Za-z0-9_$.]*" }
    FNR == NR {
      if (kind == "ir") {
        if ($0 ~ /^attributes #[0-9]+ = /) { k=$2; sub(/^[^{]*/, ""); attr[k]=$0 }
        else if ($0 ~ /^![0-9]+ = /) { k=$1; sub(/^![0-9]+ = /, ""); md[k]=$0 }
        else if ($0 ~ /^@/) { k=$1; v=$0; sub(/^[^=]*= /, "", v); gvar[substr(k,2)]=v }
        else if ($0 ~ /^define /) { if (match($0, /@("[^"]*"|[^ (]*)\(/)) isfn[substr($0, RSTART+1, RLENGTH-2)]=1 }
      } else if ($0 ~ /^(_R|_ZN)[A-Za-z0-9_$.]*:/) { isfn[substr($0, 1, index($0, ":")-1)]=1 }
      next }
    kind == "ir" && /^define / { match($0, /@("[^"]*"|[^ (]*)\(/); cur=substr($0, RSTART+1, RLENGTH-2); body=$0; nm=0; nv=0; ni[cur]=0; delete mren; delete vren; inf=1; next }
    kind == "asm" && /^(_R|_ZN)[A-Za-z0-9_$.]*:/ { cur=substr($0, 1, index($0, ":")-1); body=""; nm=0; nv=0; ni[cur]=0; delete mren; delete vren; inf=1; next }
    inf && ((kind == "ir" && /^}/) || (kind == "asm" && /^\.Lfunc_end/)) { finish(); inf=0; next }
    inf {
      l=$0
      if (kind == "ir") { if (l ~ /^ *;/) next; sub(/ *;.*$/, "", l); if (l ~ /^[^ ]/) l="%" l }
      else { if (l !~ /^\t[a-z]/) next; sub(/[ \t]*#.*$/, "", l) }
      if (kind == "asm" || l ~ /^  [^ ]/) ni[cur]++
      body = body "\n" l }
    function finish(   l, out, t, r, k, a) {
      l=body; out=""
      if (kind == "ir") {
        sub(/@("[^"]*"|[^ (]*)\(/, "@SELF(", l)
        while (match(l, /#[0-9]+/)) { a=substr(l, RSTART, RLENGTH); out=out substr(l, 1, RSTART-1) ((a in attr) ? attr[a] : a); l=substr(l, RSTART+RLENGTH) }
        l=out l; out=""
        while (match(l, /!(range|align|noundef|nonnull|prof) ![0-9]+/)) { a=substr(l, RSTART, RLENGTH); split(a, t, " "); out=out substr(l, 1, RSTART-1) t[1] " " md[t[2]]; l=substr(l, RSTART+RLENGTH) }
        l=out l; out=""
        while (match(l, /%("[^"]*"|[-A-Za-z0-9_.$]+)/)) { a=substr(l, RSTART, RLENGTH); if (!(a in vren)) vren[a]="%v" (++nv); out=out substr(l, 1, RSTART-1) vren[a]; l=substr(l, RSTART+RLENGTH) }
        l=out l; out=""
        while (match(l, /![0-9]+/)) { a=substr(l, RSTART, RLENGTH); if (!(a in mren)) mren[a]="!m" (++nm); out=out substr(l, 1, RSTART-1) mren[a]; l=substr(l, RSTART+RLENGTH) }
        l=out l; out=""
      } else {
        while (match(l, /\.L[A-Za-z_]*[0-9_]+/)) { a=substr(l, RSTART, RLENGTH); if (!(a in mren)) mren[a]=".L" (++nm); out=out substr(l, 1, RSTART-1) mren[a]; l=substr(l, RSTART+RLENGTH) }
        l=out l; out=""
      }
      r=""
      while (match(l, tokre())) {
        a=substr(l, RSTART, RLENGTH); k=(kind == "ir") ? substr(a, 2) : a
        if (k in isfn) { out=out substr(l, 1, RSTART-1) "@F"; r=r " " k }
        else out=out substr(l, 1, RSTART-1) ((k in gvar) ? "{" gvar[k] "}" : a)
        l=substr(l, RSTART+RLENGTH) }
      tmpl[cur]=out l; refs[cur]=r; fns[++nf]=cur }
    function count(c,   i, seen, n) { n=0; for (i=1; i<=nf; i++) if (!(c[fns[i]] in seen)) { seen[c[fns[i]]]=1; n++ } return n }
    END {
      for (i=1; i<=nf; i++) { g=fns[i]; m=split(refs[g], rr, " "); s=tmpl[g]; for (j=1; j<=m; j++) s=s "|" (rr[j] == g ? "SELF" : rr[j]); ex[g]=s; cl[g]=tmpl[g] }
      prev=count(cl)
      while (1) {
        for (i=1; i<=nf; i++) { g=fns[i]; m=split(refs[g], rr, " "); s=cl[g]; for (j=1; j<=m; j++) s=s "|" cl[rr[j]]; nx[g]=s }
        for (i=1; i<=nf; i++) cl[fns[i]]=nx[fns[i]]
        cur=count(cl); if (cur == prev) break; prev=cur }
      for (i=1; i<=nf; i++) { g=fns[i]; tot+=ni[g]; if (ex[g] in exrep) exsave+=ni[g]; else exrep[ex[g]]=1; if (cl[g] in clrep) clsave+=ni[g]; else clrep[cl[g]]=1 }
      printf "  functions=%d inst=%d exact-classes=%d (removes %d inst) fixpoint-classes=%d (removes %d inst)\n", nf, tot, count(ex), exsave, cur, clsave
      for (i=1; i<=nf; i++) { g=fns[i]; members[cl[g]]=members[cl[g]] "\t" g; size[cl[g]]++; if (!((cl[g] SUBSEP ex[g]) in eg)) { eg[cl[g], ex[g]]=1; neg[cl[g]]++ } }
      for (i=1; i<=nf; i++) { g=fns[i]; c=cl[g]; if (size[c] > 1 && !(c in done)) { done[c]=1; print "  class:" ni[g] " inst, exact-groups=" neg[c] members[c] } }
    }' "$1" "$1" | dm | awk -F'\t' '/^  class:/ { h=$1; sub(/^  class:/, "", h); s="  merged " (NF-1) " (" h "):"; for (i=2; i<=NF; i++) s=s (i>2 ? " | " : " ") $i; print s; next } { print }'
}
for k in pre o3nomerge o3; do echo; echo "== duplicate classes, $k IR"; classes "$out/w06.$k.ll" ir; done
echo; echo "== duplicate classes, x86 asm"; classes "$out/w06.x86.s" asm

echo; echo "== where same-shape instances differ (normalised unoptimized IR lines unique to each side)"
body() { awk -v p="$2" 'index($0, "define ") == 1 && index($0, p "(") {f=1} f {print} f && /^}/ {exit}' "$1" | grep -v '^ *;' | sed -E '1s/@[^ (]*\(/@SELF(/; s/ *;.*//; s/![0-9]+/!N/g'; }
sym() { defs "$1" | while read -r s; do printf '%s\t%s\n' "$s" "$(printf '%s\n' "$s" | dm)"; done | awk -F'\t' -v n="$2" '$2 == n {print $1; exit}'; }
pair() { a=$(sym "$out/w06.pre.ll" "$1"); b=$(sym "$out/w06.pre.ll" "$2"); echo "  -- $1  vs  $2"
  diff <(body "$out/w06.pre.ll" "$a") <(body "$out/w06.pre.ll" "$b") | grep '^[<>]' | dm | sed 's/^/    /'; }
pair 'w06::largest::<u32>' 'w06::largest::<char>'
pair 'w06::largest::<u32>' 'w06::largest::<i32>'
pair 'w06::largest::<u32>' 'w06::largest::<u64>'
pair '<alloc::vec::Vec<u32>>::push_mut' '<alloc::vec::Vec<char>>::push_mut'
pair '<alloc::raw_vec::RawVec<u32>>::grow_one' '<alloc::raw_vec::RawVec<[u8; 4]>>::grow_one'
echo; echo "== PartialOrd::gt bodies (unoptimized IR, compare lines)"
grep -n 'PartialOrd' "$out/w06.pre.ll" | grep '^[0-9]*:define' | cut -d: -f1 | while read -r n; do
  name=$(sed -n "${n}p" "$out/w06.pre.ll" | sed -E 's/.*@("[^"]*"|[^ (]*)\(.*/\1/' | dm)
  printf '  %-60s %s\n' "$name" "$(sed -n "$n,/^}/p" "$out/w06.pre.ll" | grep -E ' (icmp|fcmp) ' | sed -E 's/^ *//; s/%[A-Za-z0-9_.]+/%v/g')"; done

echo; echo "== x86 asm instructions per instance (count-asm.sh, rustc 1.99.1)"
for s in largest push_one grow_one; do "$here/count-asm.sh" "$out/w06.x86.s" "$s" | dm | sed 's/:$//; s/^/  /'; done

echo; echo "== S-graph recompute proxy: R-graph size ~ unoptimized-IR inst of the u32 instance."
echo "   instances = rustc instances; keys = kfrc answer keys INFERRED in W06 section 3 (input, not measured)"
tn=0; tk=0; ti=0
while read -r def inst keys; do
  n=$("$here/count-ir.sh" "$out/w06.pre.ll" "^$(sym "$out/w06.pre.ll" "$def")\$" | tail -1 | sed -E 's/.* inst=([0-9]+) .*/\1/')
  printf '  %-45s n=%-3s instances=%s keys=%s  specialize-nodes: per-instance=%-4s per-key=%s\n' "$def" "$n" "$inst" "$keys" $((n*inst)) $((n*keys))
  tn=$((tn+n)); ti=$((ti+n*inst)); tk=$((tk+n*keys))
done <<'T'
w06::largest::<u32> 5 5
<alloc::vec::Vec<u32>>::push_mut 4 2
<alloc::raw_vec::RawVec<u32>>::grow_one 4 2
T
echo "  TOTAL R-graph nodes(proxy)=$tn  specialize-nodes per-instance=$ti per-key=$tk"
