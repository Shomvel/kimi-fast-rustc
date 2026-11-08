#!/usr/bin/env bash
# Usage: count-ir.sh <file.ll> <name-regex>
# Per-function instruction counts for LLVM IR functions whose mangled name matches <name-regex>,
# followed by a TOTAL line. Shared by the walkthrough scripts.
set -u
awk -v pat="$2" '
  /^define / { inf=1; if (match($0, /@[^(]*\(/)) name=substr($0, RSTART+1, RLENGTH-2); else name="?";
               n=0; a=0; m=0; l=0; s=0; c=0; v=0; lp=0; ld=0; st=0; next }
  inf && /^}/ {
    if (name ~ pat) {
      printf "%5d inst  alloca=%-3d memcpy=%-3d lifetime=%-3d assume=%-3d call=%-3d invoke=%-3d landingpad=%-2d load=%-4d store=%-4d %s\n", n, a, m, l, s, c, v, lp, ld, st, name
      F++; N+=n; A+=a; M+=m; L+=l; S+=s; C+=c; V+=v; LP+=lp; LD+=ld; ST+=st
    }
    inf=0; next }
  inf && /^;/ { next }
  inf && /^  [^ ;]/ { n++ }
  inf && / = alloca / { a++ }
  inf && /@llvm\.memcpy/ { m++ }
  inf && /@llvm\.lifetime/ { l++ }
  inf && /@llvm\.assume/ { s++ }
  inf && /(^| )call / && !/@llvm\./ { c++ }
  inf && /(^| )invoke / { v++ }
  inf && / landingpad / { lp++ }
  inf && / = load / { ld++ }
  inf && /^  store / { st++ }
  END { printf "TOTAL functions=%d inst=%d alloca=%d memcpy=%d lifetime=%d assume=%d call=%d invoke=%d landingpad=%d load=%d store=%d\n", F, N, A, M, L, S, C, V, LP, LD, ST }
' "$1"
