#!/bin/bash
# Usage (from the Mac): e2b-profile.sh <name> [KFRC_REGALLOC_CHECK value]
# Requires a prior e2b-measure.sh <name> (source at /workspace/compile-<name>).
# Builds a frame-pointer + line-table copy of that backend, records
# `perf record -e instructions:u --call-graph fp` on the semver command and
# downloads demangled reports and stacks to $LOCAL/runs/compile-<name>/profile.
set -euo pipefail
name=$1
check=${2:-1}
local_root=${LOCAL:-/Users/shom/Downloads/kfrc-size-opt-20261109/compile}
H=/root/kfrc-size-opt-20261109/compile
PY=/root/SWEEnvMaker/.venv/bin/python
RUNNER=/root/kfrc-llvm-parity-lp2-20261109/tools/sandbox_runner.py
E2B=/root/kfrc-size-opt-20261109/e2b
L=$local_root/runs/compile-$name/profile
mkdir -p "$L"
cat > "$L/e2b-profile.sh" <<INNER
set -uo pipefail
W=/workspace/compile-$name
P=/workspace/results/compile-$name/profile
mkdir -p "\$P"
PERF=/usr/lib/linux-tools/6.8.0-149-generic/perf
CARGO_TARGET_DIR=/workspace/compile-fp-target CARGO_PROFILE_RELEASE_DEBUG=line-tables-only RUSTFLAGS="-Cforce-frame-pointers=yes" \
  cargo +1.94.1 build --release --manifest-path "\$W/crates/rustc_codegen_kfrc/Cargo.toml" > "\$P/fp-build.log" 2>&1 || { tail -30 "\$P/fp-build.log"; exit 1; }
cp /workspace/compile-fp-target/release/librustc_codegen_kfrc.so "\$P/fp-backend.so"
python3 -I "\$W/tools/compile-cost/semver_arms.py" --receipt /workspace/sz-results/c11-cmp1/semver/build.command.json \
  --backend "\$P/fp-backend.so" --reference-objects /workspace/results/compile-base/semver-reference/objects --out "\$P/arms" --arms plain-check$check
cmd=()
while IFS= read -r -d '' word; do cmd+=("\$word"); done < <(python3 -I "\$W/tools/compile-cost/arm_command.py" "\$P/arms/plain-check$check/arm.json")
# An empty argv would make perf record the whole machine.
[ "\${#cmd[@]}" -gt 3 ] || { echo "no semver command recorded"; exit 3; }
cd "\$P"
KFRC_REGALLOC_CHECK=$check "\$PERF" record -e instructions:u -c 100003 --call-graph fp -o perf.data -- "\${cmd[@]}" > record.log 2>&1
"\$PERF" report -i perf.data --no-children --sort symbol --stdio --percent-limit 0.05 -g none 2>/dev/null | c++filt > report-self.txt
"\$PERF" report -i perf.data --children --sort symbol --stdio --percent-limit 0.1 -g none 2>/dev/null | c++filt > report-children.txt
"\$PERF" script -i perf.data -F ip,sym 2>/dev/null | c++filt | gzip > stacks.txt.gz
rm -f perf.data fp-backend.so
cd /workspace/results
tar -czf /workspace/compile-profile-$name.tar.gz compile-$name/profile
INNER
ssh hk-linlizhi-jumpserver "mkdir -p $H/runs/$name && cat > $H/runs/$name/e2b-profile.sh" < "$L/e2b-profile.sh"
ssh hk-linlizhi-jumpserver "$PY $RUNNER run --output $E2B --command-file $H/runs/$name/e2b-profile.sh --timeout 7200 > $H/runs/$name/profile.log 2>&1; echo exit=\$?; tail -5 $H/runs/$name/profile.log; $PY $RUNNER download --output $E2B --remote /workspace/compile-profile-$name.tar.gz --local $H/runs/$name/profile.tar.gz >/dev/null"
ssh hk-linlizhi-jumpserver "cat $H/runs/$name/profile.tar.gz" > "$L/profile.tar.gz"
tar -C "$local_root/runs/compile-$name" -xzf "$L/profile.tar.gz"
