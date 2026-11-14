#!/usr/bin/env python3
"""Compiletest runner: retain exact program output and termination status."""

import hashlib
import json
import os
from pathlib import Path
import resource
import signal
import subprocess
import sys
import tempfile


def main():
    if len(sys.argv) < 2:
        raise SystemExit("usage: capture-run.py <executable> [original run flags...]")
    executable = Path(sys.argv[1]).resolve()
    capture_root = Path(os.environ["KFRC_UPSTREAM_CAPTURE_ROOT"]).resolve()
    build_root = Path(os.environ["KFRC_UPSTREAM_BUILD_ROOT"]).resolve()
    capture_root.mkdir(parents=True, exist_ok=True)
    resource.setrlimit(resource.RLIMIT_CORE, (0, 0))
    result = subprocess.run(sys.argv[1:], stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    # Each invocation has its own directory, including multiple revisions or
    # subprocess tests that execute the same binary more than once.
    directory = Path(tempfile.mkdtemp(prefix="run-", dir=capture_root))
    (directory / "stdout").write_bytes(result.stdout)
    (directory / "stderr").write_bytes(result.stderr)
    try:
        key = str(executable.relative_to(build_root))
    except ValueError:
        key = str(executable)
    record = {
        "executable": str(executable),
        "executable_key": key,
        "argv": sys.argv[1:],
        "cwd": str(Path.cwd()),
        "returncode": result.returncode,
        "stdout_sha256": hashlib.sha256(result.stdout).hexdigest(),
        "stderr_sha256": hashlib.sha256(result.stderr).hexdigest(),
        "executable_sha256": hashlib.sha256(executable.read_bytes()).hexdigest(),
    }
    (directory / "run.json").write_text(json.dumps(record, indent=2) + "\n")
    sys.stdout.buffer.write(result.stdout)
    sys.stdout.buffer.flush()
    sys.stderr.buffer.write(result.stderr)
    sys.stderr.buffer.flush()
    if result.returncode < 0:
        signum = -result.returncode
        if signum not in (signal.SIGKILL, signal.SIGSTOP):
            signal.signal(signum, signal.SIG_DFL)
        os.kill(os.getpid(), signum)
    raise SystemExit(result.returncode)


if __name__ == "__main__":
    main()
