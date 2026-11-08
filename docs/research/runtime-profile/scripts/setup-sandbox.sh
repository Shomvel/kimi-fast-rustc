#!/usr/bin/env bash
# Prepare a Linux x86_64 sandbox for the runtime-profile experiments.
#
# usage: setup-sandbox.sh WORK
#
# Installs build tools, perf and Rust 1.94.1, downloads the pinned ripgrep and
# Cargo sources, fetches their dependencies, and builds the search corpus:
#   WORK/src/{ripgrep,cargo}   pinned sources (GitHub tarballs of fixed commits)
#   WORK/src/cargo-0.95.0      the Cargo that is built and measured: cargo 0.99.0 does not
#                              build with rustc 1.94.1 (libsqlite3-sys 0.38.1 uses `cfg_select!`),
#                              0.95.0 is the Cargo of the 1.94 release and of the 2026-10 runs
#   WORK/corpus                registry sources + Rust `library/` (directory workloads)
#   WORK/big.txt               all corpus *.rs concatenated, repeated to >= 256 MiB
#   WORK/pats-lit40.txt        40 literals (-F -f workload)
#   WORK/pats-word500.txt      500 frequent identifiers (-F -f workload)
#   WORK/fingerprint.txt       toolchain, CPU, kernel, source and lockfile hashes
set -euo pipefail
WORK=${1:?usage: setup-sandbox.sh WORK}
RG_COMMIT=e89fff89ac9af12e8d4ce9d5fd07beb408ca730f   # ripgrep 15.2.0
CARGO_COMMIT=797e8a9bca276c1c9f9f738d2a20f484fa4eea9d  # cargo 0.99.0
CARGO095_SHA256=0af45a75a5265a587a8d9e4098cd2b12a07b34bcbd9eb7952ae6a62ea6d65c4d
TOOLCHAIN=1.94.1
mkdir -p "$WORK/src"
export DEBIAN_FRONTEND=noninteractive
apt-get update -qq
apt-get install -y -qq build-essential pkg-config libssl-dev zlib1g-dev linux-tools-generic libc6-dbg >/dev/null
PERF=$(ls -d /usr/lib/linux-tools/*/perf | sort -V | tail -1)
ln -sf "$PERF" /usr/local/bin/perf

export RUSTUP_HOME=/opt/rustup CARGO_HOME=/opt/cargo PATH=/opt/cargo/bin:$PATH
if ! command -v rustc >/dev/null; then
    curl -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain "$TOOLCHAIN" --no-modify-path >/dev/null
fi
rustup component add rust-src --toolchain "$TOOLCHAIN" >/dev/null

fetch_src() {  # name owner/repo commit
    local tgz="$WORK/src/$1.tar.gz"
    curl -sSfL -o "$tgz" "https://codeload.github.com/$2/tar.gz/$3"
    mkdir -p "$WORK/src/$1"
    tar -xzf "$tgz" -C "$WORK/src/$1" --strip-components=1
}
[[ -d $WORK/src/ripgrep ]] || fetch_src ripgrep BurntSushi/ripgrep "$RG_COMMIT"
[[ -d $WORK/src/cargo ]] || fetch_src cargo rust-lang/cargo "$CARGO_COMMIT"
if [[ ! -d $WORK/src/cargo-0.95.0 ]]; then
    curl -sSfL -o "$WORK/src/cargo-0.95.0.crate" https://static.crates.io/crates/cargo/cargo-0.95.0.crate
    echo "$CARGO095_SHA256  $WORK/src/cargo-0.95.0.crate" | sha256sum -c - >/dev/null
    tar -xzf "$WORK/src/cargo-0.95.0.crate" -C "$WORK/src"
fi
cargo fetch --locked --manifest-path "$WORK/src/cargo-0.95.0/Cargo.toml" >/dev/null 2>&1
cargo fetch --locked --manifest-path "$WORK/src/ripgrep/Cargo.toml" >/dev/null 2>&1
cargo fetch --locked --manifest-path "$WORK/src/cargo/Cargo.toml" >/dev/null 2>&1

if [[ ! -d $WORK/corpus ]]; then
    mkdir -p "$WORK/corpus"
    cp -r "$CARGO_HOME"/registry/src "$WORK/corpus/registry"
    cp -r "$(rustc --print sysroot)/lib/rustlib/src/rust/library" "$WORK/corpus/library"
fi
if [[ ! -f $WORK/big.txt ]]; then
    find "$WORK/corpus" -name '*.rs' -type f | LC_ALL=C sort > "$WORK/rs-files.txt"
    xargs cat < "$WORK/rs-files.txt" > "$WORK/one.txt"
    : > "$WORK/big.txt"
    while [[ $(stat -c %s "$WORK/big.txt") -lt $((256 << 20)) ]]; do cat "$WORK/one.txt" >> "$WORK/big.txt"; done
fi
# Pattern files are derived deterministically from the corpus.
grep -ohE '\b[A-Za-z_][A-Za-z0-9_]{5,}\b' "$WORK/one.txt" | LC_ALL=C sort | uniq -c \
    | LC_ALL=C sort -k1,1nr -k2,2 | awk '{print $2}' > "$WORK/idents.txt"
sed -n '1001,1040p' "$WORK/idents.txt" > "$WORK/pats-lit40.txt"
sed -n '1,500p' "$WORK/idents.txt" > "$WORK/pats-word500.txt"

{
    rustc -Vv; cargo -V
    echo "perf $("$PERF" --version)"
    uname -r; nproc; free -m | sed -n 2p
    grep -m1 'model name' /proc/cpuinfo
    for f in sse2 ssse3 sse4_2 avx2 bmi2 avx512f; do grep -qw $f /proc/cpuinfo && echo "cpuflag $f"; done
    echo "perf_event_paranoid $(cat /proc/sys/kernel/perf_event_paranoid)"
    sha256sum "$WORK"/src/*.tar.gz "$WORK"/src/*.crate "$WORK/src/ripgrep/Cargo.lock" "$WORK/src/cargo/Cargo.lock" \
        "$WORK/src/cargo-0.95.0/Cargo.lock"
    echo "corpus_files $(find "$WORK/corpus" -type f | wc -l) corpus_bytes $(du -sb "$WORK/corpus" | cut -f1)"
    echo "big_bytes $(stat -c %s "$WORK/big.txt")"
    sha256sum "$WORK/big.txt" "$WORK/pats-lit40.txt" "$WORK/pats-word500.txt"
} > "$WORK/fingerprint.txt"
cat "$WORK/fingerprint.txt"
