#!/usr/bin/env python3
"""Task-owned E2B runner. Credentials stay in SDK options on the HK host."""
import argparse
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import shlex
import time

for key in ("HTTP_PROXY", "HTTPS_PROXY", "ALL_PROXY", "http_proxy", "https_proxy", "all_proxy"):
    os.environ.pop(key, None)

from dotenv import dotenv_values
from e2b import Sandbox, Template

RUNTIME = {
    "CARGO_HOME": "/opt/cargo", "RUSTUP_HOME": "/opt/rustup",
    "PATH": "/opt/cargo/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin",
    "LD_LIBRARY_PATH": "/opt/rustup/toolchains/1.94.1-x86_64-unknown-linux-gnu/lib",
    "RUSTC_BOOTSTRAP": "1", "CARGO_BUILD_JOBS": "4",
}

parser = argparse.ArgumentParser()
parser.add_argument("action", choices=("create", "upload", "run", "download", "kill"))
parser.add_argument("--output", type=Path, required=True)
parser.add_argument("--env-file", type=Path, default=Path("/root/SWEEnvMaker/.env"))
parser.add_argument("--template", default="mirx-rust1941-linux")
parser.add_argument("--local", type=Path)
parser.add_argument("--remote")
parser.add_argument("--command-file", type=Path)
parser.add_argument("--timeout", type=int, default=3600)
args = parser.parse_args()
args.output.mkdir(parents=True, exist_ok=True)
v = dotenv_values(args.env_file)
options = dict(api_key=v["E2B_API_KEY"], api_url=v["E2B_API_URL"], domain=v["E2B_DOMAIN"], request_timeout=120)
state_path = args.output / "state.json"
state = json.loads(state_path.read_text()) if state_path.exists() else {}
if args.action == "create":
    if state.get("sandbox_id") and not state.get("killed"):
        raise SystemExit("Task already has an unreleased sandbox; reconnect with run/upload/download.")
    if not Template.exists(args.template, **options):
        raise SystemExit("Requested template is unavailable")
    sandbox = Sandbox.create(template=args.template, timeout=args.timeout, envs=RUNTIME,
        metadata={"project": "kimi-fast-rustc", "task": "poc-runnable-20261109"}, **options)
    state = dict(sandbox_id=sandbox.sandbox_id, template=args.template, timeout=args.timeout,
                 created_at=datetime.now(timezone.utc).isoformat(), killed=False)
    state_path.write_text(json.dumps(state, indent=2) + "\n")
    result = sandbox.commands.run("rustc -Vv; cargo -V; rustup component list --installed; nproc; free -m; df -h /workspace", user="root", envs=RUNTIME, timeout=60)
    (args.output / "runtime.log").write_text(result.stdout + result.stderr)
    print(result.stdout + result.stderr)
else:
    sandbox = Sandbox.connect(state["sandbox_id"], **options)
    sandbox.set_timeout(max(state["timeout"], args.timeout))
    if args.action == "upload":
        content = args.local.read_bytes()
        parts = []
        for index, offset in enumerate(range(0, len(content), 256 * 1024)):
            part = args.remote + f".part{index:05}"
            sandbox.files.write(part, content[offset:offset + 256 * 1024], user="root")
            parts.append(part)
        command = "cat " + " ".join(shlex.quote(p) for p in parts) + " > " + shlex.quote(args.remote)
        result = sandbox.commands.run(command, user="root", timeout=60)
        if result.exit_code:
            raise SystemExit(result.exit_code)
        remote_hash = sandbox.commands.run("sha256sum " + shlex.quote(args.remote), user="root", timeout=30).stdout.split()[0]
        digest = hashlib.sha256(content).hexdigest()
        if remote_hash != digest:
            raise SystemExit("Uploaded source archive hash mismatch")
        receipt = {"remote": args.remote, "bytes": len(content), "sha256": digest}
        (args.output / (args.local.name + ".upload.json")).write_text(json.dumps(receipt, indent=2) + "\n")
        print(json.dumps(receipt))
    elif args.action == "run":
        label = args.command_file.stem
        command = args.command_file.read_text()
        (args.output / (label + ".command.sh")).write_text(command)
        started = time.time()
        with (args.output / (label + ".stdout.log")).open("w") as out, (args.output / (label + ".stderr.log")).open("w") as err:
            def stdout(line):
                out.write(line + "\n"); out.flush(); print(line, flush=True)
            def stderr(line):
                err.write(line + "\n"); err.flush(); print(line, flush=True)
            try:
                result = sandbox.commands.run("bash -c " + shlex.quote(command), user="root", envs=RUNTIME,
                    timeout=args.timeout, on_stdout=stdout, on_stderr=stderr)
                exit_code = result.exit_code
            except Exception as error:
                (args.output / (label + ".result.json")).write_text(json.dumps({"exit_code": getattr(error, "exit_code", None), "error_type": type(error).__name__, "seconds": time.time() - started}, indent=2) + "\n")
                raise
        (args.output / (label + ".result.json")).write_text(json.dumps({"exit_code": exit_code, "seconds": time.time() - started}, indent=2) + "\n")
        if exit_code:
            raise SystemExit(exit_code)
    elif args.action == "download":
        expected = sandbox.commands.run("sha256sum " + shlex.quote(args.remote), user="root", timeout=30).stdout.split()[0]
        content = bytes(sandbox.files.read(args.remote, format="bytes", user="root"))
        if hashlib.sha256(content).hexdigest() != expected:
            raise SystemExit("Downloaded artifact hash mismatch")
        args.local.parent.mkdir(parents=True, exist_ok=True)
        args.local.write_bytes(content)
        print(json.dumps({"local": str(args.local), "sha256": expected, "bytes": len(content)}))
    elif args.action == "kill":
        sandbox.kill()
        state["killed"] = True
        state["killed_at"] = datetime.now(timezone.utc).isoformat()
state_path.write_text(json.dumps(state, indent=2) + "\n")
print(json.dumps(state))
