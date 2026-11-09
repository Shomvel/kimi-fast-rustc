#!/usr/bin/env bash
# Run inside an x86_64 Linux environment with Rust 1.94.1 + rustc-dev.
set -euo pipefail
repo=$(realpath "${1:?source directory}")
out=${2:?result directory}
stage=${3:-all}
mkdir -p "$out"
out=$(realpath "$out")
cd "$repo"
export RUSTC_BOOTSTRAP=1 CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS:-4}
backend="$repo/crates/rustc_codegen_kfrc/target/debug/librustc_codegen_kfrc.so"
tests="$repo/crates/rustc_codegen_kfrc/tests"
run() {
    local name=$1
    shift
    local status=0
    printf 'RUN %s\n' "$name"
    "$@" > "$out/$name.log" 2>&1 || status=$?
    printf '{"exit_code": %s}\n' "$status" > "$out/$name.result.json"
    if [ "$name" = workspace-tests ] && [ "$status" = 0 ]; then
        python3 - "$out/$name.log" <<'PY'
import pathlib, re, sys
text = pathlib.Path(sys.argv[1]).read_text()
rows = [tuple(map(int, match)) for match in re.findall(
    r"test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored;", text)]
print(f"PASS: {len(rows)} suites, {sum(row[0] for row in rows)} tests, "
      f"{sum(row[2] for row in rows)} ignored")
PY
    else
        cat "$out/$name.log"
    fi
    return "$status"
}
build_backend() {
    cargo +1.94.1 build --manifest-path "$repo/crates/rustc_codegen_kfrc/Cargo.toml"
}
if [ "$stage" = all ] || [ "$stage" = build ]; then
    rustc +1.94.1 -Vv > "$out/toolchain.txt"
    cargo +1.94.1 -V >> "$out/toolchain.txt"
    run backend-build build_backend
    sha256sum "$backend" > "$out/backend.sha256"
fi
if [ "$stage" = all ] || [ "$stage" = unit ]; then
    # FakeRustc tests create executable scripts. Serialize test execution to
    # avoid transient ETXTBSY when concurrent subprocesses inherit writer fds.
    run workspace-tests cargo +1.94.1 test --workspace -- --test-threads=1
fi
if [ "$stage" = all ] || [ "$stage" = scalar ]; then
    run scalar sh "$tests/p0-scalar/run.sh" "$backend" "$out/scalar"
    run generated sh "$tests/p0-scalar/generated/run.sh" "$backend" "$out/generated" 1 "${KFRC_GENERATED_SEEDS:-5}" 40
fi
if [ "$stage" = all ] || [ "$stage" = cargo ]; then
    run cargo bash "$tests/p0-cargo/run.sh" "$backend" "$out/cargo"
fi
if [ "$stage" = all ] || [ "$stage" = entry ]; then
    run entry sh "$tests/p0-entry/run.sh" "$backend" "$out/entry"
fi
if [ "$stage" = all ] || [ "$stage" = memory ]; then
    run memory bash "$tests/p1-memory/run.sh" "$backend" "$out/memory"
fi
if [ "$stage" = all ] || [ "$stage" = semver ]; then
    run semver bash "$tests/p3-semver/run.sh" "$backend" "$out/semver"
fi
if [ "$stage" = regex-smoke ]; then
    run regex-smoke bash "$tests/p3-semver/regex-smoke.sh" "$backend" "$out/regex-smoke"
fi
if [ "$stage" = features ]; then
    run features python3 "$repo/scripts/poc-validation/feature-probes.py" --repo "$repo" --backend "$backend" --out "$out/features"
fi
if [ "$stage" = legacy-corpus ]; then
    run legacy-corpus python3 "$repo/scripts/poc-validation/legacy-corpus-run.py" --corpus "$repo/scripts/poc-validation/fixtures/legacy-corpus" --backend "$backend" --out "$out/legacy-corpus"
fi
if [ "$stage" = cli ]; then
    run cli bash "$repo/scripts/poc-validation/cli-semantic.sh" "$repo" "$backend" "$out/cli"
fi
if [ "$stage" = semantic ]; then
    run semantic bash "$tests/semantic-ir/run.sh" "$backend" "$out/semantic"
fi
printf 'PASS: requested stage %s\n' "$stage"

if [ "$stage" = pressure ]; then
    run pressure-generate python3 "$tests/p0-scalar/generated/pressure/generate.py" "$out/pressure-inputs"
    run pressure python3 "$tests/p0-scalar/generated/pressure/run.py" --inputs "$out/pressure-inputs" --backend "$backend" --out "$out/pressure"
fi
