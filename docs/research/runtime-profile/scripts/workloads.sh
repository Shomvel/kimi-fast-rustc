#!/usr/bin/env bash
# Runtime workloads for ripgrep and Cargo.
#
# usage: workloads.sh PROJECT WORK            print the workload table
#        workloads.sh PROJECT WORK --prepare  create inputs the table needs (run once)
#
# Table (tab-separated): name, reps, normalizer, cwd, argv...  (the binary is prepended)
# normalizer: none | sort (multi-threaded output order) | json-elapsed | cargo-time
# Inputs: WORK from setup-sandbox.sh; WORK/mid.txt = first 256 MiB of WORK/big.txt;
# Cargo: WORK/lockgen (writable copy of the cargo 0.95.0 source; the 0.99.0 lockfile pins a
# yanked chacha20 0.10.1, so offline generate-lockfile cannot re-resolve it), WORK/noop-target
# (ripgrep built once by arm A's cargo so that later builds are no-ops).
set -euo pipefail
P=${1:?usage: workloads.sh PROJECT WORK [--prepare]}
W=${2:?}
if [[ ${3:-} == --prepare ]]; then
    [[ -f $W/mid.txt ]] || head -c $((256 << 20)) "$W/big.txt" > "$W/mid.txt"
    if [[ $P == cargo ]]; then
        [[ -d $W/lockgen ]] || cp -r "$W/src/cargo-0.95.0" "$W/lockgen"
        export RUSTUP_HOME=${RUSTUP_HOME:-/opt/rustup} CARGO_HOME=${CARGO_HOME:-/opt/cargo}
        export PATH=$CARGO_HOME/bin:$PATH
        (cd "$W/src/ripgrep" && "$W/bin/A/cargo" build --release --offline --locked \
            --target-dir "$W/noop-target" -j "$(nproc)" >/dev/null 2>&1)
    fi
    exit 0
fi
t() { local IFS=$'\t'; echo "$*"; }
case $P in
rg)
    R=$W/src/ripgrep C=$W/corpus M=$W/mid.txt
    t official   30 sort  "$W"  -n --no-messages 'fn\s+main' "$R"        # kfrc acceptance workload
    t dir-files   5 none  "$W"  -j1 --files --sort path "$C"             # traversal + ignore rules
    t dir-regex   5 sort  "$W"  -j1 -c 'impl\s+\w+\s+for' "$C"
    t dir-mt      5 sort  "$W"  -c 'fn\s+\w+' "$C"                       # default threads
    t lit         5 none  "$W"  -c PhantomData "$M"                      # memmem
    t lit-i       5 none  "$W"  -c -i phantomdata "$M"                   # case variants -> Teddy
    t lit40       5 none  "$W"  -c -F -f "$W/pats-lit40.txt" "$M"        # 40 literals -> Teddy
    t word500     5 none  "$W"  -c -F -f "$W/pats-word500.txt" "$M"      # 500 literals -> Aho-Corasick
    t dfa         5 none  "$W"  -c '[a-z][0-9][a-z][0-9]' "$M"           # no literal -> lazy DFA
    t suffix      5 none  "$W"  -c '[A-Z][a-z]+Error' "$M"               # suffix literal + reverse DFA
    t unicode     5 none  "$W"  -c '\b\p{Lu}\w+Error\b' "$M"
    t lines       5 none  "$W"  -n 'fn ' "$M"                            # printer: many matching lines
    t ctx         5 none  "$W"  -n -C2 'panic!\(' "$M"
    t json        5 json-elapsed "$W" --json TODO "$M"
    t help       30 none  "$W"  --help
    ;;
cargo)
    S=$W/src/cargo R=$W/src/ripgrep
    t metadata-nodeps 20 none "$S" metadata --format-version 1 --no-deps --offline --locked  # kfrc acceptance
    t metadata-full    5 none "$S" metadata --format-version 1 --offline --locked
    t tree             5 none "$S" tree --offline --locked
    t lockfile         5 none "$W/lockgen" generate-lockfile --offline
    t noop-build      10 cargo-time "$R" build --release --offline --locked --target-dir "$W/noop-target"
    t package-list    10 none "$R" package --list --allow-dirty --offline
    t build-help      30 none "$S" build --help
    ;;
*) echo "unknown project $P" >&2; exit 2 ;;
esac
