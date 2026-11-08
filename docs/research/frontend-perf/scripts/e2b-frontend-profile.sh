#!/bin/bash
# Front-end cost measurements of ripgrep and Cargo with the official rustc 1.94.1, run inside the
# E2B sandbox (template mirx-rust1941-linux, 8 vCPU / 16 GiB). Complements the 2026-10-10 release
# self-profile run (mirx/measure/profile_round.py) with check / dev / incremental scenarios and a
# -Zthreads scaling probe of `cargo check`; per-process summaries use the same JSON format.
#
# Usage (inside the sandbox; inputs uploaded to /workspace/inputs, scripts to /workspace/fe):
#   e2b-frontend-profile.sh setup            verify inputs, unpack, build summarize + perfcount, fetch, warm up
#   e2b-frontend-profile.sh profile <rg|cargo>   check, dev, dev-incr-cold, incr (self-profiled)
#   e2b-frontend-profile.sh threads <rg|cargo>   cargo check with -Zthreads=1,8,2,4,4,2,8,1 (no self-profile)
#   e2b-frontend-profile.sh pack             tar /workspace/results -> /workspace/fe-results.tar.gz (+ .sha256)
#
# Every build: clean target (except the incremental rebuild), RUSTC_BOOTSTRAP=1, RUSTC = official
# rustc, `--sysroot` of the same toolchain (as in the release run), --locked --offline -j8, wrapped in
# `/usr/bin/time -v` and perfcount (user-mode instructions/cycles inherited by the whole process tree).
# A failed build writes its result and stops the step (static audit before any rerun).
set -euo pipefail

W=/workspace
IN=$W/inputs
FE=$W/fe
R=$W/results
OFF=/opt/rustup/toolchains/1.94.1-x86_64-unknown-linux-gnu
CARGO=$OFF/bin/cargo
TOOLS=$W/measure-tools/bin
PERFCOUNT=$W/harness/perfcount
JOBS=8
EVENTS=generic-activity,query-provider
EDIT_LINE='    let _ = 1;'

project_vars() {
  case $1 in
    rg)
      SRC=$W/ripgrep; BIN=rg
      EDIT_FILE=crates/core/main.rs
      EDIT_ANCHOR='^fn eprint_nothing_searched() {$' ;;
    cargo)
      SRC=$W/cargo-0.95.0; BIN=cargo
      EDIT_FILE=src/cargo/lib.rs
      EDIT_ANCHOR='^pub fn display_warning_with_error(warning: &str, err: &Error, shell: &mut Shell) {$' ;;
    *) echo "unknown project: $1" >&2; exit 2 ;;
  esac
}

# run_build OUT TARGET INCREMENTAL(0|default) RUSTFLAGS EXTRA_JSON SCENARIO PROFILE_DIR CARGO_ARGS...
# PROFILE_DIR is where this build writes its .mm_profdata files ("-" for none); it must be empty before the build.
# Runs one cargo invocation for project $PROJECT and writes OUT/{command.txt,build.log,time.txt,perf.json,result.json}.
run_build() {
  local out=$1 target=$2 incr=$3 flags=$4 extra=$5 scenario=$6 profdir=$7
  shift 7
  mkdir -p "$out"
  local envs=(env -u CARGO_ENCODED_RUSTFLAGS -u CARGO_BUILD_RUSTFLAGS -u CARGO_INCREMENTAL
    -u RUSTC_WRAPPER -u RUSTC_WORKSPACE_WRAPPER
    "RUSTC=$OFF/bin/rustc" RUSTC_BOOTSTRAP=1 "CARGO_TARGET_DIR=$target" "RUSTFLAGS=$flags")
  if [ "$incr" = 0 ]; then envs+=(CARGO_INCREMENTAL=0); fi
  local cmd=("$CARGO" "$@" --bin "$BIN" --locked --offline -j "$JOBS")
  { printf 'cwd: %s\n' "$SRC"; printf '%q ' "${envs[@]}" "${cmd[@]}"; printf '\n'; } > "$out/command.txt"
  uptime > "$out/load-before.txt"
  rm -f "$out/perf.json" "$out/time.txt"
  local status=0 start end
  start=$EPOCHREALTIME
  (cd "$SRC" && "${envs[@]}" /usr/bin/time -v -o "$out/time.txt" "$PERFCOUNT" "$out/perf.json" -- "${cmd[@]}") \
    > "$out/build.log" 2>&1 || status=$?
  end=$EPOCHREALTIME
  [ -s "$out/perf.json" ] || echo '{}' > "$out/perf.json"
  [ -f "$out/time.txt" ] || : > "$out/time.txt"
  local profiles=0 processes=0
  if [ "$profdir" != - ]; then
    profiles=$(find "$profdir" -maxdepth 1 -name '*.mm_profdata' | wc -l)
    processes=$(find "$profdir" -maxdepth 1 -name '*.mm_profdata' ! -name '___-*' | wc -l)
  fi
  jq -c -n -f "$FE/result.jq" --arg project "$PROJECT" --arg scenario "$scenario" \
    --argjson start "$start" --argjson end "$end" --argjson exit "$status" \
    --argjson profiles "$profiles" --argjson rustc_processes "$processes" --argjson extra "$extra" \
    --rawfile time "$out/time.txt" --slurpfile perf "$out/perf.json" > "$out/result.json"
  cat "$out/result.json"
  if [ "$status" != 0 ]; then
    tail -40 "$out/build.log"
    echo "build failed (exit $status): stop for a static audit" >&2
    exit 1
  fi
}

# Move profiles written to a shared directory (incremental scenarios) into OUT/profiles.
collect_profiles() {
  local from=$1 out=$2
  mkdir -p "$out/profiles"
  find "$from" -maxdepth 1 -name '*.mm_profdata' -exec mv -t "$out/profiles" {} +
}

# Reduce every OUT/profiles/*.mm_profdata with `summarize summarize --json` and merge into OUT/summaries.json.
summarize_profiles() {
  local out=$1
  local dir=$out/profiles
  du -sb "$dir" | cut -f1 > "$out/profiles-bytes.txt"
  find "$dir" -maxdepth 1 -name '*.mm_profdata' -print0 \
    | xargs -0 -r -P "$JOBS" -n 1 "$TOOLS/summarize" summarize --json > "$out/summarize.log" 2>&1
  local traces jsons
  traces=$(find "$dir" -maxdepth 1 -name '*.mm_profdata' | wc -l)
  jsons=$(find "$dir" -maxdepth 1 -name '*.json' | wc -l)
  if [ "$traces" != "$jsons" ]; then
    echo "summarize produced $jsons summaries for $traces profiles" >&2
    tail -20 "$out/summarize.log" >&2
    exit 1
  fi
  local files=()
  mapfile -d '' files < <(find "$dir" -maxdepth 1 -name '*.json' -print0 | sort -z)
  jq -c -n -f "$FE/summaries-merge.jq" "${files[@]}" > "$out/summaries.json"
  local keys
  keys=$(jq -f "$FE/summaries-count.jq" "$out/summaries.json")
  if [ "$keys" != "$traces" ]; then
    echo "summaries.json has $keys processes, expected $traces" >&2
    exit 1
  fi
  rm -rf "$dir"
}

copy_timings() {
  local target=$1 out=$2
  if [ -f "$target/cargo-timings/cargo-timing.html" ]; then
    cp "$target/cargo-timings/cargo-timing.html" "$out/cargo-timing.html"
  fi
}

profile_flags() {
  printf -- '--sysroot %s -Zself-profile=%s -Zself-profile-events=%s' "$OFF" "$1" "$EVENTS"
}

do_setup() {
  mkdir -p "$R/fingerprint"
  (cd "$IN" && sha256sum -c inputs.sha256) | tee "$R/fingerprint/inputs-check.txt"
  cp "$IN/inputs.sha256" "$R/fingerprint/inputs.sha256"
  (cd "$FE" && sha256sum ./*) > "$R/fingerprint/scripts.sha256"
  cd "$W"
  tar xzf "$IN/ripgrep-src.tar.gz"
  tar xzf "$IN/cargo-src.tar.gz"
  tar xzf "$IN/rs-harness.tar.gz"
  sha256sum ripgrep/Cargo.lock cargo-0.95.0/Cargo.lock > "$R/fingerprint/cargo-lock.sha256"
  sha256sum "ripgrep/crates/core/main.rs" "cargo-0.95.0/src/cargo/lib.rs" > "$R/fingerprint/edit-files.sha256"
  "$OFF/bin/rustc" -vV > "$R/fingerprint/rustc-vV.txt"
  "$CARGO" -vV > "$R/fingerprint/cargo-vV.txt"
  sha256sum "$OFF/bin/rustc" "$OFF/bin/cargo" "$OFF"/lib/*.so* \
    > "$R/fingerprint/toolchain.sha256"
  {
    echo "== uname"; uname -a
    echo "== os-release"; cat /etc/os-release
    echo "== nproc"; nproc
    echo "== lscpu"; lscpu
    echo "== free -m"; free -m
    echo "== df"; df -h "$W"
    echo "== perf_event_paranoid"; cat /proc/sys/kernel/perf_event_paranoid
    echo "== cpu.max"; if [ -f /sys/fs/cgroup/cpu.max ]; then cat /sys/fs/cgroup/cpu.max; else echo absent; fi
    echo "== jq"; jq --version
    echo "== gcc"; gcc --version | head -1
    echo "== time"; dpkg-query -W time || true
  } > "$R/fingerprint/host.txt" 2>&1
  # measureme summarize, offline from the vendored source; its #![deny(warnings)] rejects the
  # x86_64-only `unsafe { __cpuid() }` with Rust 1.94, so lints are capped at warn (as in the release run).
  tar xzf "$W/harness/measureme-vendored.tar.gz" -C "$W"
  (cd "$W/measureme" && RUSTFLAGS=--cap-lints=warn "$CARGO" build --release --offline --locked -p summarize \
    --target-dir "$W/mm-target") > "$R/measureme-build.log" 2>&1 \
    || { tail -30 "$R/measureme-build.log"; exit 1; }
  mkdir -p "$TOOLS"
  cp "$W/mm-target/release/summarize" "$TOOLS/"
  (cd "$W/measureme" && git rev-parse HEAD 2>/dev/null || true) > "$R/fingerprint/measureme-rev.txt"
  sha256sum "$TOOLS/summarize" > "$R/fingerprint/summarize.sha256"
  gcc -O2 -Wall -o "$PERFCOUNT" "$W/harness/perfcount.c"
  sha256sum "$W/harness/perfcount.c" "$PERFCOUNT" > "$R/fingerprint/perfcount.sha256"
  "$PERFCOUNT" "$R/fingerprint/perf-probe.json" -- true
  echo "perf probe: $(cat "$R/fingerprint/perf-probe.json")"
  for p in rg cargo; do
    project_vars "$p"
    (cd "$SRC" && "$CARGO" fetch --locked) > "$R/fetch-$p.log" 2>&1 || { tail -30 "$R/fetch-$p.log"; exit 1; }
  done
  # Unmeasured warm-up (page cache for registry sources and the toolchain): one check per project.
  for p in rg cargo; do
    PROJECT=$p
    project_vars "$p"
    rm -rf "$W/target/warmup-$p"
    run_build "$R/warmup/$p" "$W/target/warmup-$p" 0 "--sysroot $OFF" '{}' warmup-check - check
    rm -rf "$W/target/warmup-$p"
  done
  nproc; free -m; uptime
}

do_profile() {
  PROJECT=$1
  project_vars "$PROJECT"
  local base=$R/profile/$PROJECT out target prof
  # Clean, non-incremental check and dev build.
  for scenario in check dev; do
    local sub=check
    [ "$scenario" = dev ] && sub=build
    out=$base/$scenario
    target=$W/target/$PROJECT-$scenario
    rm -rf "$target" "$out"
    mkdir -p "$out/profiles"
    run_build "$out" "$target" 0 "$(profile_flags "$out/profiles")" '{}' "$scenario" "$out/profiles" "$sub" --timings
    copy_timings "$target" "$out"
    summarize_profiles "$out"
    rm -rf "$target"
  done
  # Cold dev build with the default incremental setting, then a one-line leaf edit and an
  # incremental rebuild. Both use the same RUSTFLAGS (same profile directory) so that Cargo's
  # fingerprints match; profiles are moved out after each build.
  prof=$W/prof-incr/$PROJECT
  target=$W/target/$PROJECT-incr
  rm -rf "$prof" "$target" "$base/dev-incr-cold" "$base/incr"
  mkdir -p "$prof"
  out=$base/dev-incr-cold
  run_build "$out" "$target" default "$(profile_flags "$prof")" '{}' dev-incr-cold "$prof" build --timings
  copy_timings "$target" "$out"
  collect_profiles "$prof" "$out"
  summarize_profiles "$out"
  out=$base/incr
  mkdir -p "$out" "$W/edit-backup/$PROJECT"
  local file=$SRC/$EDIT_FILE backup line before
  backup=$W/edit-backup/$PROJECT/$(basename "$EDIT_FILE")
  cp -p "$file" "$backup"
  before=$(sha256sum < "$file")
  if [ "$(grep -c -- "$EDIT_ANCHOR" "$file")" != 1 ]; then
    echo "edit anchor not unique in $EDIT_FILE" >&2
    exit 1
  fi
  line=$(grep -n -- "$EDIT_ANCHOR" "$file" | cut -d: -f1)
  sed -i "${line}a\\${EDIT_LINE}" "$file"
  diff -u --label "a/$EDIT_FILE" --label "b/$EDIT_FILE" "$backup" "$file" > "$out/edit.patch" || true
  if [ "$(grep -c '^+    let _ = 1;$' "$out/edit.patch")" != 1 ]; then
    echo "edit not applied as expected" >&2
    cat "$out/edit.patch" >&2
    exit 1
  fi
  sleep 1
  run_build "$out" "$target" default "$(profile_flags "$prof")" '{}' incr "$prof" build --timings
  copy_timings "$target" "$out"
  collect_profiles "$prof" "$out"
  summarize_profiles "$out"
  grep -E '^ *(Compiling|Checking|Finished)' "$out/build.log" > "$out/rebuilt-units.txt" || true
  cp -p "$backup" "$file"
  if [ "$(sha256sum < "$file")" != "$before" ]; then
    echo "failed to restore $EDIT_FILE" >&2
    exit 1
  fi
  rm -rf "$target" "$prof"
  df -h "$W" | tail -1
}

do_threads() {
  PROJECT=$1
  project_vars "$PROJECT"
  local base=$R/threads/$PROJECT target=$W/target/$PROJECT-threads n k
  rm -rf "$base"
  mkdir -p "$base"
  declare -A seen=()
  for n in 1 8 2 4 4 2 8 1; do
    k=$(( ${seen[$n]:-0} + 1 ))
    seen[$n]=$k
    rm -rf "$target"
    run_build "$base/t$n-s$k" "$target" 0 "--sysroot $OFF -Zthreads=$n" "{\"threads\": $n, \"sample\": $k}" threads - check
    rm -rf "$target"
  done
  local files=()
  mapfile -d '' files < <(find "$base" -mindepth 2 -maxdepth 2 -name result.json -print0 | sort -z)
  jq -s -f "$FE/threads-table.jq" "${files[@]}" > "$base/threads.json"
  cat "$base/threads.json"
}

do_pack() {
  cd "$W"
  tar --exclude=profiles -czf "$W/fe-results.tar.gz" results
  (cd "$W" && sha256sum fe-results.tar.gz > fe-results.tar.gz.sha256)
  ls -l "$W/fe-results.tar.gz"
  cat "$W/fe-results.tar.gz.sha256"
}

step=${1:-}
case $step in
  setup) do_setup ;;
  profile) do_profile "${2:?project}" ;;
  threads) do_threads "${2:?project}" ;;
  pack) do_pack ;;
  *) echo "usage: $0 setup | profile <rg|cargo> | threads <rg|cargo> | pack" >&2; exit 2 ;;
esac
