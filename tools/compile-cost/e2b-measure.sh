#!/bin/bash
# Usage (from the Mac): e2b-measure.sh <name> [arms] [extra semver_arms.py args...]
# Snapshots REPO, builds its release backend on the shared E2B sandbox
# (CARGO_TARGET_DIR=/workspace/sz-target-compile), runs semver_arms.py against the
# reference objects REFERENCE (default: compile-base release objects) and downloads
# /workspace/results/compile-<name> to $LOCAL/runs/compile-<name>.
set -euo pipefail
name=$1
arms=${2:-total-check1,total-check0,stages-check1,stages-check0}
shift $(( $# >= 2 ? 2 : $# ))
script_repo=$(cd "$(dirname "$0")/../.." && pwd)
repo=${REPO:-$script_repo}
local_root=${LOCAL:-/Users/shom/Downloads/kfrc-size-opt-20261109/compile}
reference=${REFERENCE:-/workspace/results/compile-base/semver-reference/objects}
backend=${BACKEND:-}
H=/root/kfrc-size-opt-20261109/compile
PY=/root/SWEEnvMaker/.venv/bin/python
RUNNER=/root/kfrc-llvm-parity-lp2-20261109/tools/sandbox_runner.py
E2B=/root/kfrc-size-opt-20261109/e2b
L=$local_root/runs/compile-$name
mkdir -p "$L"
tar -C "$repo" -czf "$L/source.tar.gz" --exclude target --exclude __pycache__ crates scripts tools Cargo.toml Cargo.lock
(cd "$repo" && git rev-parse HEAD > "$L/source-head.txt" && git status --short > "$L/source-status.txt")
printf '%s\n' "$repo" > "$L/source-root.txt"
shasum -a 256 "$L/source.tar.gz" > "$L/source.sha256"
extra=""
for word in "$@"; do extra="$extra '$word'"; done
cat > "$L/e2b-run.sh" <<INNER
set -uo pipefail
name=$name
W=/workspace/compile-\$name
out=/workspace/results/compile-\$name
mkdir -p "\$out"
test -d "\$W" && mv "\$W" "/workspace/compile-trash-\$name-\$\$"
mkdir -p "\$W"
tar -xzf /workspace/compile-\$name.tar.gz -C "\$W"
cp /workspace/backend-Cargo.lock "\$W/crates/rustc_codegen_kfrc/Cargo.lock"
df -h /workspace | tail -1
backend="$backend"
if [ -z "\$backend" ]; then
  CARGO_TARGET_DIR=/workspace/sz-target-compile cargo +1.94.1 build --release --manifest-path "\$W/crates/rustc_codegen_kfrc/Cargo.toml" > "\$out/backend-build.log" 2>&1 || { tail -40 "\$out/backend-build.log"; exit 1; }
  cp /workspace/sz-target-compile/release/librustc_codegen_kfrc.so "\$out/backend.so"
  backend=\$out/backend.so
fi
sha256sum "\$backend" | tee "\$out/backend.sha256"
python3 -I "\$W/tools/compile-cost/semver_arms.py" --receipt /workspace/sz-results/c11-cmp1/semver/build.command.json \
  --backend "\$backend" --reference-objects "$reference" --out "\$out" --arms "$arms" $extra
status=\$?
tar -C /workspace/results -czf /workspace/compile-results-\$name.tar.gz --exclude='*/objects' --exclude='*.rlib' --exclude='backend.so' compile-\$name
exit \$status
INNER
ssh hk-linlizhi-jumpserver "mkdir -p $H/runs/$name"
ssh hk-linlizhi-jumpserver "cat > $H/runs/$name/source.tar.gz" < "$L/source.tar.gz"
ssh hk-linlizhi-jumpserver "cat > $H/runs/$name/e2b-run.sh" < "$L/e2b-run.sh"
status=0
ssh hk-linlizhi-jumpserver "$PY $RUNNER upload --output $E2B --local $H/runs/$name/source.tar.gz --remote /workspace/compile-$name.tar.gz >/dev/null && $PY $RUNNER run --output $E2B --command-file $H/runs/$name/e2b-run.sh --timeout 7200 > $H/runs/$name/run.log 2>&1; run_status=\$?; echo exit=\$run_status; tail -12 $H/runs/$name/run.log; $PY $RUNNER download --output $E2B --remote /workspace/compile-results-$name.tar.gz --local $H/runs/$name/results.tar.gz >/dev/null; exit \$run_status" | tee "$L/run-tail.log" || status=$?
ssh hk-linlizhi-jumpserver "cat $H/runs/$name/results.tar.gz" > "$L/results.tar.gz"
ssh hk-linlizhi-jumpserver "cat $H/runs/$name/run.log" > "$L/run.log"
tar -C "$L" -xzf "$L/results.tar.gz"
exit "$status"
