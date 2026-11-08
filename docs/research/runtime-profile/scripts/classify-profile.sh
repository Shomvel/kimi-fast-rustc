#!/usr/bin/env bash
# Group flat perf profiles (profile-hotspots.sh *.sym.txt) into hot-path categories.
#
# usage: classify-profile.sh PROFILE_DIR [TOPN]
#
# Prints, per workload, the self-cycle share of each category (first matching rule
# wins, rules below), then the TOPN (default 8) symbols with their category.
# Categories name the mechanism a backend must reproduce, e.g. "memchr SIMD" is
# code reached through core::arch runtime dispatch.
set -euo pipefail
DIR=${1:?usage: classify-profile.sh PROFILE_DIR [TOPN]}
TOPN=${2:-8}
RULES=$(cat <<'EOF'
memchr::arch::x86_64::(avx2|sse2)	memchr/memmem SIMD (core::arch)
memchr::	memchr/memmem generic
aho_corasick::packed	Teddy / packed (core::arch)
aho_corasick::	Aho-Corasick automaton
regex_automata::hybrid	regex lazy DFA
regex_automata::dfa::	regex DFA (dense/onepass)
regex_automata::nfa::thompson::pikevm	regex PikeVM
regex_automata::nfa::thompson::backtrack	regex backtracker
regex_automata::(meta|util::prefilter|util::search)	regex meta + prefilter glue
regex_automata::|regex_syntax::|regex::	regex compile/other
grep_searcher::	searcher (lines, buffers)
grep_printer::|termcolor::	printer
grep_regex::|grep_matcher::|grep_cli::|^grep::|^rg::|lexopt::	rg glue + args
ignore::|globset::|walkdir::|same_file::	dir walk + ignore/glob
serde_json::|serde::|serde_core::|serde_derive	serde / JSON
toml_edit::|toml::|toml_parser::|toml_datetime::|toml_writer::|winnow::|serde_spanned::	TOML parse
cargo::core::resolver|cargo::core::dependency|semver::|cargo::core::summary|cargo::core::package_id	resolver + semver
cargo::util::interning|InternedString	string interning
cargo::sources|cargo::util::toml|cargo::core::manifest|cargo::core::workspace|cargo::util::context	manifest/workspace/sources
cargo::core::compiler|cargo::util::fingerprint|filetime::|cargo::ops::cargo_compile	compile graph + fingerprints
cargo::ops|cargo::core::|cargo::util::|^cargo::|cargo_util|cargo_platform|crates_io::	cargo other
sha2::|sha1|blake3|hex::|crc32fast	hashing
git2::|libgit2|^git_|gix	git
curl|ssl|SSL|crypto	network/TLS
clap|anstream|anstyle|textwrap	CLI parsing/help
adler|flate2|inflate|zlib|tar::	decompress
hashbrown::|HashMap|HashSet|indexmap::|BTreeMap|btree	hash/btree maps
core::fmt|alloc::fmt|fmt::Write|Display|Debug>::fmt|itoa::|zmij::|ryu::	formatting
^(__)?(memmove|memcpy|memset|memcmp|memchr|memrchr|strlen|bcmp|strcmp)|__mem[a-z]+_(avx|evex|sse)	libc mem*/str*
malloc|free|realloc|_int_|tcache|calloc|cfree|alloc::alloc|__rust_alloc|__rust_dealloc	allocator
alloc::|core::|std::|<.*as core	std/core other
EOF
)
for f in "$DIR"/*.sym.txt; do
    w=$(basename "$f" .sym.txt)
    out=$(RULES="$RULES" awk -v topn="$TOPN" '
    BEGIN { n = split(ENVIRON["RULES"], r, "\n"); for (i = 1; i <= n; i++) { split(r[i], p, "\t"); re[i] = p[1]; cat[i] = p[2] } }
    match($0, /^ *[0-9.]+%/) {
        pct = substr($0, RSTART, RLENGTH); gsub(/[ %]/, "", pct)
        sym = $0; sub(/^ *[0-9.]+% +(\[[.k]\] +)?/, "", sym)
        c = "other"
        for (i = 1; i <= n; i++) if (sym ~ re[i]) { c = cat[i]; break }
        share[c] += pct; total += pct
        if (++rank <= topn) printf "T\t%06d\t| %.2f%% | %s | %s |\n", rank, pct, c, substr(sym, 1, 110)
    }
    END {
        for (c in share) printf "C\t%09.3f\t| %s | %.1f%% |\n", share[c], c, share[c]
        printf "R\t\t| (symbols below 0.1%%) | %.1f%% |\n", 100 - total
    }' "$f")
    echo "## $w"
    echo "| category | self cycles |"; echo "|---|---:|"
    grep '^C' <<< "$out" | sort -t$'\t' -k2,2r | cut -f3
    grep '^R' <<< "$out" | cut -f3
    echo
    echo "| share | category | top symbol |"; echo "|---:|---|---|"
    grep '^T' <<< "$out" | sort -t$'\t' -k2,2 | cut -f3
    echo
done
