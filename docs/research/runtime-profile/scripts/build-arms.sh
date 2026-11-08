#!/usr/bin/env bash
# Build one project once per attribution arm and collect the binaries.
#
# usage: build-arms.sh WORK PROJECT [ARM...]
#
#   WORK     prepared by setup-sandbox.sh
#   PROJECT  rg | cargo
#   ARM      default: all arms from arm-flags.sh --list
#
# Each arm: fresh target dir, `cargo build --release --offline --target x86_64-unknown-linux-gnu`
# with the arm's RUSTFLAGS. `--target` keeps RUSTFLAGS off build scripts and proc macros
# (they build at opt-level 0, where e.g. -regalloc=basic is rejected), as in bench-yicloud.sh.
# Patch arms (S2/S0) build a copy of the source with `--config patch.crates-io.*`
# pointing at make-simd-patches.sh output (the copy's Cargo.lock is updated offline).
# Outputs:
#   WORK/bin/<ARM>/<bin>         binaries (same file name in every arm: clap prints argv[0])
#   WORK/bin/<PROJECT>-builds.tsv  arm, exit, wall/user/sys s, maxrss KiB, bytes, sha256, rustflags
#   WORK/logs/<PROJECT>-<ARM>.log  build log
set -uo pipefail
WORK=${1:?usage: build-arms.sh WORK PROJECT [ARM...]}
PROJECT=${2:?}
shift 2
HERE=$(cd "$(dirname "$0")" && pwd)
export RUSTUP_HOME=${RUSTUP_HOME:-/opt/rustup} CARGO_HOME=${CARGO_HOME:-/opt/cargo}
export PATH=$CARGO_HOME/bin:$PATH CARGO_TERM_COLOR=never
TARGET=x86_64-unknown-linux-gnu
case $PROJECT in
    rg)    SRC=$WORK/src/ripgrep PKG=ripgrep BIN=rg EXTRA=() ;;
    cargo) SRC=$WORK/src/cargo-0.95.0 PKG=cargo BIN=cargo EXTRA=() ;;   # see setup-sandbox.sh
    *) echo "unknown project $PROJECT" >&2; exit 2 ;;
esac
ARMS=("$@")
[[ ${#ARMS[@]} -gt 0 ]] || read -r -a ARMS <<< "$(bash "$HERE/arm-flags.sh" --list)"
mkdir -p "$WORK/bin" "$WORK/logs" "$WORK/target"
TSV=$WORK/bin/$PROJECT-builds.tsv
[[ -f $TSV ]] || printf 'arm\texit\twall_s\tuser_s\tsys_s\tmaxrss_kib\tbytes\tsha256\trustflags\n' > "$TSV"
REG=$(ls -d "$CARGO_HOME"/registry/src/index.crates.io-* | head -1)
for arm in "${ARMS[@]}"; do
    kind=$(bash "$HERE/arm-flags.sh" --kind "$arm")
    flags=$(bash "$HERE/arm-flags.sh" "$arm")
    T=$WORK/target/$PROJECT-$arm
    rm -rf "$T"
    src=$SRC lock=(--locked) patch=()
    if [[ $kind == patch ]]; then
        bash "$HERE/make-simd-patches.sh" "$REG" "$SRC/Cargo.lock" "$WORK/simd-patches" > "$WORK/logs/simd-patches-$PROJECT.txt"
        src=$WORK/src/$PROJECT-$arm
        [[ -d $src ]] || cp -r "$SRC" "$src"
        lock=()
        patch=(--config "patch.crates-io.memchr.path=\"$WORK/simd-patches/$arm/memchr\""
               --config "patch.crates-io.aho-corasick.path=\"$WORK/simd-patches/$arm/aho-corasick\"")
    fi
    log=$WORK/logs/$PROJECT-$arm.log
    echo "== $PROJECT $arm RUSTFLAGS='$flags'"
    /usr/bin/time -f 'wall=%e user=%U sys=%S maxrss=%M' -o "$T.time" \
        env RUSTFLAGS="$flags" CARGO_TARGET_DIR="$T" \
        cargo build --manifest-path "$src/Cargo.toml" --release --offline "${lock[@]}" "${EXTRA[@]}" \
        "${patch[@]}" --target "$TARGET" -p "$PKG" --bin "$BIN" -j "$(nproc)" > "$log" 2>&1
    rc=$?
    if [[ $kind == patch ]]; then diff "$SRC/Cargo.lock" "$src/Cargo.lock" > "$WORK/logs/$PROJECT-$arm.lock.diff"; fi
    read -r wall user sys rss < <(sed -E 's/[a-z]+=//g' "$T.time" | tail -1)
    bytes=- sha=-
    if [[ $rc == 0 ]]; then
        mkdir -p "$WORK/bin/$arm"
        cp "$T/$TARGET/release/$BIN" "$WORK/bin/$arm/$BIN"
        bytes=$(stat -c %s "$WORK/bin/$arm/$BIN")
        sha=$(sha256sum "$WORK/bin/$arm/$BIN" | cut -c1-16)
    else
        tail -20 "$log"
    fi
    printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' "$arm" "$rc" "$wall" "$user" "$sys" "$rss" "$bytes" "$sha" "$flags" | tee -a "$TSV"
    # Keep A's target for the cargo no-op workload; drop the rest to bound disk use.
    [[ $arm == A ]] || rm -rf "$T"
done
