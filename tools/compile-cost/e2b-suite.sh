#!/bin/bash
# Usage (from the Mac): e2b-suite.sh <name> [arms]
# Runs crate_arms.py on E2B with the backend built by `e2b-measure.sh <name>`
# (/workspace/results/compile-<name>/backend.so) against the reference objects of
# compile-base (created by `e2b-suite.sh base`), and downloads the results to
# $LOCAL/runs/compile-<name>/suite.
set -euo pipefail
name=$1
arms=${2:-total-check0,stages-check0,total-check1}
local_root=${LOCAL:-/Users/shom/Downloads/kfrc-size-opt-20261109/compile}
reference=${REFERENCE_ROOT:-/workspace/results/compile-base/suite-reference}
H=/root/kfrc-size-opt-20261109/compile
PY=/root/SWEEnvMaker/.venv/bin/python
RUNNER=/root/kfrc-llvm-parity-lp2-20261109/tools/sandbox_runner.py
E2B=/root/kfrc-size-opt-20261109/e2b
L=$local_root/runs/compile-$name/suite
mkdir -p "$L"
cat > "$L/e2b-suite.sh" <<INNER
set -uo pipefail
out=/workspace/results/compile-$name/suite
mkdir -p "\$out"
mkdir -p /workspace/compile-tools-$name
tar -xzf /workspace/compile-tools-$name.tar.gz -C /workspace/compile-tools-$name
T=/workspace/compile-tools-$name
python3 -I "\$T/crate_arms.py" --commands "\$T/suite-commands.tsv" \
  --backend /workspace/results/compile-$name/backend.so --reference-root "$reference" \
  --out "\$out" --arms "$arms" --llvm-wrapper /workspace/candidate28/crates/rustc_codegen_llvm_profile/target/release/librustc_codegen_llvm_profile.so
status=\$?
tar -C /workspace/results -czf /workspace/compile-suite-$name.tar.gz --exclude='*/objects' --exclude='*.rlib' compile-$name/suite
exit \$status
INNER
here=$(cd "$(dirname "$0")" && pwd)
tar -C "$here" -czf "$L/tools.tar.gz" crate_arms.py suite-commands.tsv
ssh hk-linlizhi-jumpserver "mkdir -p $H/runs/$name && cat > $H/runs/$name/e2b-suite.sh" < "$L/e2b-suite.sh"
ssh hk-linlizhi-jumpserver "cat > $H/runs/$name/tools.tar.gz" < "$L/tools.tar.gz"
status=0
ssh hk-linlizhi-jumpserver "$PY $RUNNER upload --output $E2B --local $H/runs/$name/tools.tar.gz --remote /workspace/compile-tools-$name.tar.gz >/dev/null && $PY $RUNNER run --output $E2B --command-file $H/runs/$name/e2b-suite.sh --timeout 7200 > $H/runs/$name/suite.log 2>&1; run_status=\$?; echo exit=\$run_status; tail -3 $H/runs/$name/suite.log; $PY $RUNNER download --output $E2B --remote /workspace/compile-suite-$name.tar.gz --local $H/runs/$name/suite.tar.gz >/dev/null; exit \$run_status" || status=$?
ssh hk-linlizhi-jumpserver "cat $H/runs/$name/suite.tar.gz" > "$L/suite.tar.gz"
ssh hk-linlizhi-jumpserver "cat $H/runs/$name/suite.log" > "$L/suite.log"
tar -C "$local_root/runs/compile-$name" -xzf "$L/suite.tar.gz"
exit "$status"
