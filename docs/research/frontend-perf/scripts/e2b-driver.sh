#!/bin/bash
# Host-side driver for e2b-frontend-profile.sh through the rust-build-profile E2B entry
# (mirx/e2b/local-direct.py -> manage.py, SDK e2b==2.34.0, template mirx-rust1941-linux).
# The E2B credentials file stays in RBP_ROOT and is read only by manage.py; it is never uploaded
# and never written to RUN_DIR (state.json there holds only the sandbox ID and lease).
#
# Usage: e2b-driver.sh RBP_ROOT RUN_DIR ACTION [ARGS]
#   RBP_ROOT  rust-build-profile checkout (mirx/e2b/.venv, local-direct.py, source tarballs, rs/rs-harness.tar.gz)
#   RUN_DIR   new local run directory (state, logs, downloaded results)
#   ACTION    create | upload | run STEP [PROJECT] | download | kill
#             STEP is a step of e2b-frontend-profile.sh: setup, profile, threads, pack
set -euo pipefail

if [ $# -lt 3 ]; then
  sed -n '2,12p' "$0" >&2
  exit 2
fi
RBP=$1
RUN=$2
ACTION=$3
shift 3
HERE=$(cd "$(dirname "$0")" && pwd)
E2B=$RBP/mirx/e2b
PY=$E2B/.venv/bin/python
ENTRY=$E2B/local-direct.py
TEMPLATE=mirx-rust1941-linux
LEASE=10800
mkdir -p "$RUN"

entry() {
  "$PY" "$ENTRY" "$@" --output "$RUN"
}

case $ACTION in
  create)
    entry create --template "$TEMPLATE" --timeout "$LEASE" 2>&1 | tee -a "$RUN/create.log"
    ;;
  upload)
    inputs=("$E2B/ripgrep-src.tar.gz" "$E2B/cargo-src.tar.gz" "$E2B/rs/rs-harness.tar.gz")
    for f in "${inputs[@]}"; do
      (cd "$(dirname "$f")" && shasum -a 256 "$(basename "$f")")
    done > "$RUN/inputs.sha256"
    scripts=("$HERE/e2b-frontend-profile.sh" "$HERE"/*.jq)
    (cd "$HERE" && shasum -a 256 e2b-frontend-profile.sh e2b-driver.sh ./*.jq) > "$RUN/scripts.sha256"
    for f in "${inputs[@]}" "$RUN/inputs.sha256"; do
      entry upload --local "$f" --remote "/workspace/inputs/$(basename "$f")" 2>&1 | tee -a "$RUN/upload.log"
    done
    for f in "${scripts[@]}"; do
      entry upload --local "$f" --remote "/workspace/fe/$(basename "$f")" 2>&1 | tee -a "$RUN/upload.log"
    done
    ;;
  run)
    step=${1:?step}
    project=${2:-}
    label=fe-$step${project:+-$project}
    mkdir -p "$RUN/cmd"
    printf '#!/bin/bash\nexec bash /workspace/fe/e2b-frontend-profile.sh %s %s\n' "$step" "$project" > "$RUN/cmd/$label.sh"
    entry run --command-file "$RUN/cmd/$label.sh" --timeout "$LEASE" 2>&1 | tee "$RUN/$label.console.log"
    ;;
  download)
    entry download --remote /workspace/fe-results.tar.gz --local "$RUN/fe-results.tar.gz" 2>&1 | tee -a "$RUN/download.log"
    entry download --remote /workspace/fe-results.tar.gz.sha256 --local "$RUN/fe-results.tar.gz.sha256" 2>&1 | tee -a "$RUN/download.log"
    (cd "$RUN" && shasum -a 256 -c fe-results.tar.gz.sha256)
    tar xzf "$RUN/fe-results.tar.gz" -C "$RUN"
    ;;
  kill)
    entry kill 2>&1 | tee -a "$RUN/kill.log"
    ;;
  *)
    echo "unknown action: $ACTION" >&2
    exit 2
    ;;
esac
