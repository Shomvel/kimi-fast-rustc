#!/usr/bin/env bash
# Usage: count-asm.sh <file.s> <symbol-substring>
# Prints the instruction count of each x86 function whose label contains <symbol-substring>.
set -u
awk -v pat="$2" '
  /^[A-Za-z_.$][A-Za-z0-9_.$]*:/ && $0 !~ /^\.L/ { if (f) printf "%5d asm-inst %s\n", n, name; f = (index($0, pat) > 0); name=$0; n=0; next }
  f && /^\.Lfunc_end/ { printf "%5d asm-inst %s\n", n, name; f=0 }
  f && /^\t[a-z]/ { n++ }
' "$1"
