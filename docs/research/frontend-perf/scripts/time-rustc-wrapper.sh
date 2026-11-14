#!/usr/bin/env bash
# RUSTC_WRAPPER that records each rustc process's rusage with macOS `/usr/bin/time -l`
# (instructions retired cover only the timed process itself, so every rustc is wrapped).
# Needs KFRC_TIME_DIR. Cargo calls: <wrapper> <rustc> <args...>
set -euo pipefail
exec /usr/bin/time -l -o "$KFRC_TIME_DIR/rustc-$$-$RANDOM.txt" "$@"
