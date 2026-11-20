#!/usr/bin/env python3
"""Capture reproducible, credential-free rustc inputs while executing LLVM."""
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import time

argv = sys.argv[1:]
allowed = {
    "PATH", "LD_LIBRARY_PATH", "CARGO_HOME", "RUSTUP_HOME", "RUSTC_BOOTSTRAP",
    "OUT_DIR", "CARGO_MANIFEST_DIR", "CARGO_MANIFEST_PATH", "CARGO_PRIMARY_PACKAGE",
    "CARGO_CRATE_NAME", "CARGO_BIN_NAME", "CARGO", "RUSTC", "RUSTDOC", "HOST", "TARGET",
    "OPT_LEVEL", "PROFILE", "DEBUG", "NUM_JOBS", "DEP_OPENSSL_INCLUDE",
}
env = {k: v for k, v in os.environ.items()
       if k in allowed or k.startswith(("CARGO_PKG_", "CARGO_FEATURE_", "DEP_", "CARGO_CFG_"))}
build_env_source = None
if "OUT_DIR" in os.environ:
    output = Path(os.environ["OUT_DIR"]).parent / "output"
    if output.exists():
        declared = set()
        for line in output.read_text().splitlines():
            for prefix in ("cargo:rustc-env=", "cargo::rustc-env="):
                if line.startswith(prefix):
                    declared.add(line[len(prefix):].split("=", 1)[0])
        for key in declared:
            if key in os.environ:
                env[key] = os.environ[key]
        build_env_source = {"path": str(output), "sha256": hashlib.sha256(output.read_bytes()).hexdigest(),
                            "declared_names": sorted(declared)}
# Environment remains in the live subprocess; only the allowlist is persisted.
record = {"argv": argv, "cwd": os.getcwd(), "env": env, "started_ns": time.time_ns()}
if build_env_source:
    record["build_script_env_source"] = build_env_source
capture = Path(os.environ["KFRC_CAPTURE_DIR"])
capture.mkdir(parents=True, exist_ok=True)
digest = hashlib.sha256(json.dumps(record, sort_keys=True).encode()).hexdigest()[:16]
path = capture / (str(record["started_ns"]) + "-" + digest + ".json")
path.write_text(json.dumps(record, indent=2) + "\n")
result = subprocess.run(argv)
record["exit_code"] = result.returncode
record["finished_ns"] = time.time_ns()
path.write_text(json.dumps(record, indent=2) + "\n")
sys.exit(result.returncode)
