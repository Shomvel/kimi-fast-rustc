#!/usr/bin/env bash
# Group flat perf profiles (profile-hotspots.sh *.sym.txt) into hot-path categories.
#
# usage: classify-profile.sh PROFILE_DIR [TOPN]
#        classify-profile.sh PROFILE_DIR matrix [MIN_PCT]
#
# matrix: one Markdown table, workloads × categories, keeping categories whose share
# reaches MIN_PCT (default 5) in at least one workload; the rest are summed as "rest".
#
# Input lines look like "  12.34%  libc.so.6  [.] malloc"; rules match "<dso> <symbol>".
# Prints, per workload, the self-cycle share of each category (first matching rule
# wins, rules below), then the TOPN (default 8) symbols with their category.
# Categories name the mechanism a backend must reproduce, e.g. "memchr SIMD" is
# code reached through core::arch runtime dispatch.
set -euo pipefail
DIR=${1:?usage: classify-profile.sh PROFILE_DIR [TOPN]}
TOPN=${2:-8}
MIN=${3:-5}
MATRIX=$(mktemp)
RULES=$(cat <<'EOF'
^\[unknown\] |\[k\] 0xffffffff	kernel entry (sample skid)
core::core_arch::	core::arch intrinsic called out of line
memchr::arch::x86_64::	memchr/memmem SIMD (core::arch)
memchr::	memchr/memmem generic
core::slice::memchr|TwoWaySearcher|core::str::pattern	std memchr/str search (SWAR)
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
core::hash::sip|sha2::|sha1|blake3|hex::|crc32fast|rustc_stable_hash|siphasher	hashing (SipHash etc.)
git2::|libgit2|^git_|gix	git
^libcrypto|^libssl|^libcurl|^libssh|^libgit2|^libz	C libraries (OpenSSL etc.)
pthread_rwlock|pthread_once|__pthread	libc pthread (called by C libraries)
curl::|openssl|git2::	Rust bindings to C libraries
clap|anstream|anstyle|textwrap	CLI parsing/help
adler|flate2|inflate|zlib|tar::	decompress
hashbrown::|HashMap|HashSet|indexmap::|BTreeMap|btree	hash/btree maps
core::fmt|alloc::fmt|fmt::Write|Display|Debug>::fmt|itoa::|zmij::|ryu::	formatting
(__)?(memmove|memcpy|memset|memcmp|memchr|memrchr|strlen|bcmp|strcmp)|__mem[a-z]+_(avx|evex|sse)|__str[a-z]+_(avx|evex|sse)	libc mem*/str*
malloc|free|realloc|_int_|tcache|calloc|cfree|alloc::alloc|__rust_alloc|__rust_dealloc|arena_get|unlink_chunk	allocator
^libc[^ ]* .*(readdir|opendir|closedir|getdents|stat|open|close|read|write|mmap|munmap|madvise|syscall|fcntl|ioctl|lseek)	libc syscall wrappers / dirent
^libc	libc other
^ld-linux	dynamic loader
alloc::|core::|std::|<.*as core	std/core other
EOF
)
for f in "$DIR"/*.sym.txt; do
    w=$(basename "$f" .sym.txt)
    [[ $TOPN == matrix ]] && TOPN_RUN=0 || TOPN_RUN=$TOPN
    out=$(RULES="$RULES" awk -v topn="$TOPN_RUN" '
    BEGIN { n = split(ENVIRON["RULES"], r, "\n"); for (i = 1; i <= n; i++) { split(r[i], p, "\t"); re[i] = p[1]; cat[i] = p[2] } }
    match($0, /^ *[0-9.]+%/) {
        pct = substr($0, RSTART, RLENGTH); gsub(/[ %]/, "", pct)
        sym = $0; sub(/^ *[0-9.]+% +/, "", sym); sub(/ +\[[.k]\] +/, " ", sym)   # "<dso> <symbol>"
        # Legacy-mangled names perf left alone (".llvm.NNN" suffix): decode the path.
        if (match(sym, /_ZN[0-9]/)) {
            head = substr(sym, 1, RSTART - 1); rest = substr(sym, RSTART + 3); path = ""
            while (match(rest, /^[0-9]+/)) {
                len = substr(rest, 1, RLENGTH) + 0
                path = path (path == "" ? "" : "::") substr(rest, RLENGTH + 1, len)
                rest = substr(rest, RLENGTH + 1 + len)
            }
            gsub(/\$LT\$/, "<", path); gsub(/\$GT\$/, ">", path); gsub(/\$u20\$/, " ", path)
            gsub(/\$RF\$/, "\\&", path); gsub(/\$C\$/, ",", path); gsub(/\.\./, "::", path)
            sub(/::h[0-9a-f]+$/, "", path); sub(/^_</, "<", path)
            sym = head path
        }
        c = "other"
        for (i = 1; i <= n; i++) if (sym ~ re[i]) { c = cat[i]; break }
        share[c] += pct; total += pct
        if (++rank <= topn) printf "T\t%06d\t| %.2f%% | %s | %s |\n", rank, pct, c, substr(sym, 1, 110)
    }
    END {
        for (c in share) printf "C\t%09.3f\t| %s | %.1f%% |\n", share[c], c, share[c]
        printf "R\t\t| (symbols below 0.1%%) | %.1f%% |\n", 100 - total
    }' "$f")
    if [[ $TOPN == matrix ]]; then
        grep '^C' <<< "$out" | awk -F'\t' -v w="$w" '{split($3, p, " \\| "); sub(/^\| /, "", p[1]); print w "\t" p[1] "\t" $2 + 0}' >> "$MATRIX"
        continue
    fi
    echo "## $w"
    echo "| category | self cycles |"; echo "|---|---:|"
    grep '^C' <<< "$out" | sort -t$'\t' -k2,2r | cut -f3
    grep '^R' <<< "$out" | cut -f3
    echo
    echo "| share | category | top symbol |"; echo "|---:|---|---|"
    grep '^T' <<< "$out" | sort -t$'\t' -k2,2 | cut -f3
    echo
done
if [[ $TOPN == matrix ]]; then
    awk -F'\t' -v min="$MIN" '
        { if (!($1 in sw)) { sw[$1] = 1; ws[++nw] = $1 }
          v[$1 SUBSEP $2] = $3; if ($3 > mx[$2]) mx[$2] = $3; tot[$2] += $3 }
        END {
            for (c in mx) if (mx[c] >= min) keep[++nk] = c
            for (i = 2; i <= nk; i++) { t = keep[i]; for (j = i - 1; j >= 1 && tot[keep[j]] < tot[t]; j--) keep[j + 1] = keep[j]; keep[j + 1] = t }
            printf "| workload |"; for (k = 1; k <= nk; k++) printf " %s |", keep[k]; print " rest |"
            printf "|---|"; for (k = 1; k <= nk; k++) printf "---:|"; print "---:|"
            for (i = 1; i <= nw; i++) {
                w = ws[i]; s = 0; printf "| %s |", w
                for (k = 1; k <= nk; k++) { x = v[w SUBSEP keep[k]] + 0; s += x; printf " %.1f |", x }
                printf " %.1f |\n", 100 - s
            }
        }' "$MATRIX"
    :
fi
rm -f "$MATRIX"
