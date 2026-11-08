#!/bin/bash
# Oracle probe for docs/migration/rustc/traits/test-cases.md.
# Runs every c*.rs with the default solver and with -Znext-solver=globally
# (needs RUSTC_BOOTSTRAP=1 on a stable 1.94.1 rustc) and prints exit codes
# plus the set of error codes. Usage: run.sh [rustc]; outputs go to a temp dir.
set -u
here="$(cd "$(dirname "$0")" && pwd)"
rustc_bin="${1:-rustc}"
out="$(mktemp -d)"
"$rustc_bin" -Vv | grep -E '^(release|commit-hash):'
cd "$here"
for f in c*.rs; do
  b="${f%.rs}"
  "$rustc_bin" --edition 2021 --emit=metadata -o "$out/$b.cur.rmeta" "$f" >"$out/$b.cur.txt" 2>&1; ec1=$?
  RUSTC_BOOTSTRAP=1 "$rustc_bin" --edition 2021 -Znext-solver=globally --emit=metadata \
    -o "$out/$b.next.rmeta" "$f" >"$out/$b.next.txt" 2>&1; ec2=$?
  c1=$(grep -o '^error\[E[0-9]*\]' "$out/$b.cur.txt" | sort -u | tr -d 'error[]' | tr '\n' ' ')
  c2=$(grep -o '^error\[E[0-9]*\]' "$out/$b.next.txt" | sort -u | tr -d 'error[]' | tr '\n' ' ')
  printf '%-28s cur=%s [%s]  next=%s [%s]\n' "$b" "$ec1" "$c1" "$ec2" "$c2"
done
echo "diagnostics: $out"
