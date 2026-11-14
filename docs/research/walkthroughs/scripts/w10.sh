#!/usr/bin/env bash
# Usage: w10.sh <workdir>   — runs w10.rs as a binary, then emits MIR/IR/asm as a library
# (pub functions stay external, so O3 does not rewrite their signatures) and prints partition-relevant facts.
# Env: RUSTC (rustc 1.94.1, host), RUSTC_X86 (a rustc with x86_64-unknown-linux-musl std).
set -u
here=$(cd "$(dirname "$0")" && pwd); out=${1:?workdir}; mkdir -p "$out"
RUSTC=${RUSTC:-rustc}; RUSTC_X86=${RUSTC_X86:-rustc}
f="-Copt-level=3 -Ccodegen-units=1"
"$RUSTC" $f -o "$out/w10" "$here/w10.rs" && "$out/w10" && echo "run: ok"
lib="$f --crate-type=lib"
"$RUSTC" $lib --emit=mir -o "$out/w10.mir" "$here/w10.rs"
"$RUSTC" $lib -Cno-prepopulate-passes --emit=llvm-ir -o "$out/w10.pre.ll" "$here/w10.rs"
"$RUSTC" $lib --emit=llvm-ir -o "$out/w10.o3.ll" "$here/w10.rs"
"$RUSTC_X86" $lib --target x86_64-unknown-linux-musl --emit=asm -o "$out/w10.x86.s" "$here/w10.rs"
"$RUSTC_X86" -vV | sed -n 's/^release: /x86 rustc release: /p'
fns='frozen cell mixed mixed_nocall shallow pair refcell unique_fence unique_acquire frozen_fence unique_atomic local_atomic pinned'
sym() { printf '_ZN3w10%d%s17h' "${#1}" "$1"; }
fn_body() { awk -v p="$2" '/^define / && index($0, p) {f=1} f {print} f && /^}/ {f=0}' "$1"; }
# pointer-parameter attributes that matter for partitions, one line per pointer parameter
# pointer-parameter attributes that matter for partitions, one line per pointer parameter
ptr_attrs() {
  fn_body "$1" "$(sym "$2")" | head -1 | sed 's/^[^@]*@[^(]*(//; s/) unnamed_addr.*//' | awk '{gsub(/, ptr /, "\nptr "); print}' | grep '^ptr ' \
    | awk -v n="$2" '{a=""; for (i=1;i<=NF;i++) if ($i ~ /^(noalias|readonly|writeonly|dereferenceable|nonnull)/) a=a" "$i; printf "  %-15s %-6s%s\n", n, $NF, (a==""?" (none)":a)}'
}
echo "== rustc-emitted pointer parameter attributes (unoptimized IR)"
for n in $fns; do ptr_attrs "$out/w10.pre.ll" "$n"; done
echo "== O3 IR: non-atomic load/store, atomics, fences, non-intrinsic calls per function"
for n in $fns; do
  b=$(fn_body "$out/w10.o3.ll" "$(sym "$n")" | grep -v "^ *;")
  if [ -z "$b" ]; then echo "  $n: $(grep "^@$(sym "$n")[0-9a-f]*E = .*alias" "$out/w10.o3.ll" | sed -E 's/.*ptr @_ZN3w10[0-9]+([a-z_]+)17h.*/merged by MergeFunctions into: \1/')"; continue; fi
  printf "  %-15s load=%d store=%d atomic-load/store=%d rmw=%d fence=%d call=%d\n" "$n" \
    "$(grep ' = load ' <<<"$b" | grep -vc ' atomic ')" "$(grep '^  store ' <<<"$b" | grep -vc ' atomic ')" \
    "$(grep -cE ' (load|store) atomic ' <<<"$b")" "$(grep -c ' atomicrmw ' <<<"$b")" "$(grep -c '^  fence ' <<<"$b")" \
    "$(grep -E ' (tail )?call ' <<<"$b" | grep -vc '@llvm\.')"
done
echo "== O3 IR bodies"
for n in refcell unique_fence unique_acquire frozen_fence unique_atomic local_atomic shallow; do
  echo "  [$n]"; fn_body "$out/w10.o3.ll" "$(sym "$n")" | sed -n '2,$p' | grep -vE '^ *(;|$)|^}|llvm\.lifetime' \
    | sed -E 's/, !(dbg|noalias|alias.scope|noundef|range|nonnull|prof) ![0-9]+//g; s/@_ZN[0-9A-Za-z_$.]*E/@<sym>/g; s/^ */    /; s/ +;.*//' | cut -c1-100
done
echo "== MIR: calls left in refcell after MIR inlining"
awk '/^fn refcell/ {f=1} f && / = .*\(.*\) -> / {print} f && /^}/ {f=0}' "$out/w10.mir" | sed -E 's/^ +/  /'
echo "== x86 asm instruction counts"
for n in $fns; do "$here/count-asm.sh" "$out/w10.x86.s" "$(printf '%d%s' "${#n}" "$n")"; done
echo "== unique_fence: non-atomic stores kept across fence(Release), O3 IR, by compiler (host target)"
"$RUSTC_X86" $lib --emit=llvm-ir -o "$out/w10.o3.x86rustc.ll" "$here/w10.rs"
for c in "$RUSTC:$out/w10.o3.ll" "$RUSTC_X86:$out/w10.o3.x86rustc.ll"; do
  ll=${c#*:}; v=$("${c%%:*}" -vV | awk '/^release:/ {r=$2} /^LLVM version:/ {l=$3} END {print "rustc " r " / LLVM " l}')
  echo "  $v: stores=$(awk '/^define .*unique_fence/ {f=1} f && /^  store / {n++} f && /^}/ {f=0} END {print n+0}' "$ll")"
done
