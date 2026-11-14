#!/usr/bin/env python3
"""Build and invoke the byte-exact, pinned upstream compiletest executable.

This adapter supplies bootstrap's CLI configuration. Test directives, expected
outputs, revision discovery, execution and verdicts belong to compiletest.
"""

from __future__ import annotations

import argparse
from dataclasses import dataclass
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import signal
import shutil
import subprocess
import sys
import tarfile
import tomllib


COMMIT = "e408947bfd200af42db322daf0fadfe7e26d3bd1"
ARCHIVE_SHA256 = "063f7b364a2f0dee6d8f6733486ea7b6510c79682129448b2e6bd4ffc3e2a8fe"
ARCHIVE_ROOT = f"rust-{COMMIT}/"
SOURCE_DIRS = (
    "src/tools/compiletest/",
    "src/build_helper/",
    "src/tools/miropt-test-tools/",
)
SOURCE_FILES = {
    "Cargo.lock",
    "src/stage0",  # build_helper::stage0_parser uses include_str!("../../stage0").
    "tests/auxiliary/minicore.rs",
    "COPYRIGHT",
    "LICENSE-APACHE",
    "LICENSE-MIT",
}
WORKSPACE = '''[workspace]
resolver = "2"
members = ["src/tools/compiletest", "src/build_helper", "src/tools/miropt-test-tools"]
'''


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def write_json(path: Path, value: object) -> None:
    path.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n")


def prepare_workspace(archive: Path, output: Path) -> dict:
    """Copy original tool/path dependencies into a small independent workspace.

    Every upstream file stays byte-identical. Only the workspace-root manifest
    is generated; its three members retain their original relative paths.
    """
    archive, output = archive.resolve(), output.resolve()
    if sha256(archive) != ARCHIVE_SHA256:
        raise ValueError("source archive does not match the pinned SHA-256")
    if output.exists() and any(output.iterdir()):
        raise FileExistsError(f"workspace is not empty: {output}")
    output.mkdir(parents=True, exist_ok=True)
    copied = []
    with tarfile.open(archive, "r:gz") as source:
        for member in source:
            if not member.name.startswith(ARCHIVE_ROOT) or not member.isfile():
                continue
            relative = member.name[len(ARCHIVE_ROOT):]
            if relative not in SOURCE_FILES and not relative.startswith(SOURCE_DIRS):
                continue
            parts = PurePosixPath(relative)
            if parts.is_absolute() or ".." in parts.parts:
                raise ValueError(f"unsafe archive member: {member.name}")
            contents = source.extractfile(member)
            if contents is None:
                raise ValueError(f"archive member has no contents: {member.name}")
            target = output.joinpath(*parts.parts)
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes(contents.read())
            copied.append({"path": relative, "sha256": sha256(target)})
    missing = SOURCE_FILES.difference(item["path"] for item in copied)
    if missing:
        raise ValueError(f"archive lacks required files: {sorted(missing)}")
    (output / "Cargo.toml").write_text(WORKSPACE)
    result = {
        "upstream_commit": COMMIT,
        "archive_sha256": ARCHIVE_SHA256,
        "workspace": str(output),
        "generated_manifest_sha256": sha256(output / "Cargo.toml"),
        "upstream_files": sorted(copied, key=lambda item: item["path"]),
    }
    write_json(output / "source-provenance.json", result)
    return result


def build_tool(
    workspace: Path,
    target_dir: Path,
    *,
    toolchain: str = "1.94.1",
    cargo: str = "cargo",
    jobs: int = 4,
) -> dict:
    """Build with the normal LLVM host toolchain; run this in E2B, not locally."""
    workspace, target_dir = workspace.resolve(), target_dir.resolve()
    provenance = json.loads((workspace / "source-provenance.json").read_text())
    if provenance["upstream_commit"] != COMMIT:
        raise ValueError("workspace provenance has a different upstream commit")
    if sha256(workspace / "Cargo.toml") != provenance["generated_manifest_sha256"]:
        raise ValueError("modified generated workspace manifest")
    logs = workspace / "build-evidence"
    logs.mkdir(exist_ok=True)
    if (logs / "build.json").exists():
        attempt = 1
        while (logs / f"attempt-{attempt:03}").exists():
            attempt += 1
        retained_logs = logs / f"attempt-{attempt:03}"
        retained_logs.mkdir()
        for previous in logs.iterdir():
            if previous.is_file():
                shutil.copyfile(previous, retained_logs / previous.name)
    original_lock = logs / "upstream.Cargo.lock"
    for item in provenance["upstream_files"]:
        # The active Cargo.lock is a generated workspace artifact. Keep the
        # byte-exact original separately once Cargo prunes unrelated members.
        retained = original_lock if item["path"] == "Cargo.lock" and original_lock.exists() else workspace / item["path"]
        if sha256(retained) != item["sha256"]:
            raise ValueError(f"modified upstream build input: {item['path']}")
    if not original_lock.exists():
        shutil.copyfile(workspace / "Cargo.lock", original_lock)
    shutil.copyfile(original_lock, workspace / "Cargo.lock")
    # An isolated workspace cannot retain all 643 original workspace package
    # entries under --locked. Cargo's --workspace update preserves already
    # locked registry versions while reconciling just the workspace graph.
    lock_argv = [cargo, f"+{toolchain}", "update", "--workspace", "--manifest-path",
                 str(workspace / "Cargo.toml")]
    with (logs / "lock.stdout").open("wb") as stdout, (logs / "lock.stderr").open("wb") as stderr:
        lock_update = subprocess.run(lock_argv, cwd=workspace, stdout=stdout, stderr=stderr)
    binary = target_dir / "release/compiletest"
    result = {"lock_command": lock_argv, "original_lock_sha256": sha256(original_lock),
              "lockfile_sha256": sha256(workspace / "Cargo.lock"),
              "lock_stdout": str(logs / "lock.stdout"), "lock_stderr": str(logs / "lock.stderr"),
              "binary": str(binary), "upstream_commit": COMMIT}
    if lock_update.returncode:
        result.update({"phase": "lock-update", "returncode": lock_update.returncode})
        write_json(logs / "build.json", result)
        return result
    original_packages = tomllib.loads(original_lock.read_text())["package"]
    generated_packages = tomllib.loads((workspace / "Cargo.lock").read_text())["package"]
    pinned_registry = {
        (package["name"], package["version"], package["source"]): package.get("checksum")
        for package in original_packages if "source" in package
    }
    unpinned = [package for package in generated_packages if "source" in package and
                ((package["name"], package["version"], package["source"]) not in pinned_registry
                 or pinned_registry[(package["name"], package["version"], package["source"])] != package.get("checksum"))]
    result["original_lock_packages"] = len(original_packages)
    result["generated_lock_packages"] = len(generated_packages)
    result["unpinned_registry_packages"] = unpinned
    shutil.copyfile(workspace / "Cargo.lock", logs / "workspace.Cargo.lock")
    if unpinned:
        result.update({"phase": "lock-validation", "returncode": 1})
        write_json(logs / "build.json", result)
        return result
    argv = [cargo, f"+{toolchain}", "build", "--locked", "--release",
            "--manifest-path", str(workspace / "Cargo.toml"), "-p", "compiletest",
            "--bin", "compiletest", "--target-dir", str(target_dir), "-j", str(jobs)]
    with (logs / "build.stdout").open("wb") as stdout, (logs / "build.stderr").open("wb") as stderr:
        completed = subprocess.run(argv, cwd=workspace, stdout=stdout, stderr=stderr)
    result.update({"phase": "build", "command": argv, "returncode": completed.returncode,
                   "stdout": str(logs / "build.stdout"), "stderr": str(logs / "build.stderr")})
    if completed.returncode == 0:
        result["binary_sha256"] = sha256(binary)
    result["lockfile_sha256"] = sha256(workspace / "Cargo.lock")
    write_json(logs / "build.json", result)
    return result


@dataclass(frozen=True)
class Toolchain:
    rustc: Path
    sysroot: Path
    host: str
    target: str
    version: str
    llvm_version: str
    commit: str = COMMIT


def probe_toolchain(rustc: Path) -> Toolchain:
    """Query the real rustc executable supplied by rustup which --toolchain."""
    rustc = rustc.resolve()
    version = subprocess.check_output([str(rustc), "-vV"], text=True)
    fields = dict(line.split(": ", 1) for line in version.splitlines() if ": " in line)
    if fields.get("commit-hash") != COMMIT:
        raise ValueError(f"rustc commit differs from pinned upstream: {fields.get('commit-hash')}")
    sysroot = Path(subprocess.check_output([str(rustc), "--print", "sysroot"], text=True).strip())
    return Toolchain(rustc, sysroot, fields["host"], fields["host"], version,
                     fields.get("LLVM version", ""), fields["commit-hash"])


def command(
    tool: Path,
    toolchain: Toolchain,
    source_root: Path,
    build_root: Path,
    source: str,
    *,
    backend: Path | None = None,
    rustc_flags: tuple[str, ...] = (),
    runner: Path | None = None,
    fail_fast: bool = True,
    minicore: Path | None = None,
) -> list[str]:
    """Return official CLI arguments for one complete UI file, all revisions.

    `source` is the manifest path `ui/family/file.rs`; `source_root` contains
    `tests/ui`. The exact filter is relative to that suite: `family/file.rs`.
    """
    path = PurePosixPath(source)
    if path.is_absolute() or ".." in path.parts or len(path.parts) < 2 or path.parts[0] != "ui":
        raise ValueError(f"expected a suite-relative UI source path: {source}")
    source_root, build_root = source_root.resolve(), build_root.resolve()
    sysroot = toolchain.sysroot.resolve()
    host_lib = sysroot / "lib"
    target_lib = sysroot / "lib/rustlib" / toolchain.target / "lib"
    if minicore is None:
        minicore = source_root / "tests/auxiliary/minicore.rs"
    release = next((line[9:] for line in toolchain.version.splitlines()
                    if line.startswith("release: ")), "1.94.1")
    channel = "nightly" if "nightly" in release else "beta" if "beta" in release else "stable"
    options = {
        "compile-lib-path": str(host_lib), "run-lib-path": str(target_lib),
        "rustc-path": str(toolchain.rustc), "query-rustc-path": str(toolchain.rustc),
        "python": sys.executable, "src-root": str(source_root),
        "src-test-suite-root": str(source_root / "tests/ui"),
        "build-root": str(build_root), "build-test-suite-root": str(build_root / "ui"),
        "sysroot-base": str(sysroot), "stage": "2", "stage-id": f"stage2-{toolchain.host}",
        "mode": "ui", "suite": "ui", "host": toolchain.host, "target": toolchain.target,
        "cc": shutil.which("cc") or "cc", "cxx": shutil.which("c++") or "c++",
        "cflags": "", "cxxflags": "", "llvm-components": "x86", "channel": channel,
        "nightly-branch": "master", "git-merge-commit-email": "bors@rust-lang.org",
        "minicore-path": str(minicore.resolve()), "jobs": "1",
    }
    if toolchain.llvm_version:
        options["llvm-version"] = toolchain.llvm_version
    if backend is not None:
        options["override-codegen-backend"] = str(backend.resolve())
    if runner is not None:
        if " " in str(runner.resolve()):
            raise ValueError("compiletest splits --runner at spaces; runner path must contain none")
        options["runner"] = str(runner.resolve())
    argv = [str(tool.resolve()), *(f"--{key}={value}" for key, value in options.items())]
    # optmulti stores each flag separately and applies it to host/target auxiliaries.
    for flag in rustc_flags:
        argv.extend([f"--host-rustcflags={flag}", f"--target-rustcflags={flag}"])
    argv.extend(["--force-rerun", "--exact"])
    if fail_fast:
        argv.append("--fail-fast")
    argv.append(str(PurePosixPath(*path.parts[1:])))
    return argv


def invoke(
    tool: Path,
    toolchain: Toolchain,
    source_root: Path,
    build_root: Path,
    source: str,
    *,
    logs: Path,
    backend: Path | None = None,
    rustc_flags: tuple[str, ...] = (),
    runner: Path | None = None,
    fail_fast: bool = True,
    minicore: Path | None = None,
    env_overrides: dict[str, str] | None = None,
    timeout: float = 120,
) -> dict:
    """Run the original tool and retain raw output and its official JSON events."""
    logs, build_root = logs.resolve(), build_root.resolve()
    logs.mkdir(parents=True, exist_ok=True)
    build_root.mkdir(parents=True, exist_ok=True)
    stdout_path, stderr_path = logs / "compiletest.stdout", logs / "compiletest.stderr"
    if stdout_path.exists() or stderr_path.exists():
        raise FileExistsError(f"invocation logs already exist in {logs}")
    if timeout <= 0:
        raise ValueError("invocation timeout must be positive")
    argv = command(tool, toolchain, source_root, build_root, source, backend=backend,
                   rustc_flags=rustc_flags, runner=runner, fail_fast=fail_fast, minicore=minicore)
    env = os.environ.copy()
    env.update({"RUSTC_BOOTSTRAP": "1", "RUST_TEST_THREADS": "1", "NO_COLOR": "1"})
    env.pop("RUSTC_TEST_FAIL_FAST", None)
    env.update(env_overrides or {})
    timed_out = False
    with stdout_path.open("wb") as stdout, stderr_path.open("wb") as stderr:
        with subprocess.Popen(argv, env=env, stdout=stdout, stderr=stderr,
                              start_new_session=True) as process:
            try:
                returncode = process.wait(timeout=timeout)
            except subprocess.TimeoutExpired:
                timed_out = True
                # A test may have spawned children; terminate the whole fresh
                # process group rather than leaving a workload in the sandbox.
                try:
                    os.killpg(process.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
                returncode = process.wait()
    events, non_json_lines = [], []
    for line in stdout_path.read_text(errors="replace").splitlines():
        try:
            event = json.loads(line)
        except json.JSONDecodeError:
            non_json_lines.append(line)
            continue
        if isinstance(event, dict) and event.get("type") in {"suite", "test"}:
            events.append(event)
        else:
            non_json_lines.append(line)
    write_json(logs / "events.json", events)
    result = {"command": argv, "returncode": returncode, "source": source,
              "stdout": str(stdout_path), "stderr": str(stderr_path),
              "events": events, "non_json_stdout": non_json_lines,
              "timed_out": timed_out, "timeout_seconds": timeout,
              "build_root": str(build_root), "upstream_commit": COMMIT}
    write_json(logs / "invocation.json", result)
    return result


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="action", required=True)
    prepare = sub.add_parser("prepare", help="extract exact upstream tool sources")
    prepare.add_argument("--archive", type=Path, required=True)
    prepare.add_argument("--output", type=Path, required=True)
    build = sub.add_parser("build", help="build the official tool in E2B")
    build.add_argument("--workspace", type=Path, required=True)
    build.add_argument("--target-dir", type=Path, required=True)
    build.add_argument("--toolchain", default="1.94.1")
    build.add_argument("--cargo", default="cargo")
    build.add_argument("--jobs", type=int, default=4)
    run = sub.add_parser("run", help="invoke one exact upstream UI file")
    run.add_argument("--tool", type=Path, required=True)
    run.add_argument("--rustc", type=Path, required=True)
    run.add_argument("--source-root", type=Path, required=True)
    run.add_argument("--build-root", type=Path, required=True)
    run.add_argument("--source", required=True)
    run.add_argument("--logs", type=Path, required=True)
    run.add_argument("--backend", type=Path)
    run.add_argument("--runner", type=Path)
    run.add_argument("--minicore", type=Path)
    run.add_argument("--rustc-flag", action="append", default=[])
    run.add_argument("--no-fail-fast", action="store_true")
    run.add_argument("--timeout", type=float, default=120)
    args = parser.parse_args()
    if args.action == "prepare":
        result = prepare_workspace(args.archive, args.output)
    elif args.action == "build":
        result = build_tool(args.workspace, args.target_dir, toolchain=args.toolchain,
                            cargo=args.cargo, jobs=args.jobs)
    else:
        result = invoke(args.tool, probe_toolchain(args.rustc), args.source_root,
                        args.build_root, args.source, logs=args.logs, backend=args.backend,
                        runner=args.runner, minicore=args.minicore,
                        rustc_flags=tuple(args.rustc_flag), fail_fast=not args.no_fail_fast,
                        timeout=args.timeout)
    print(json.dumps(result, indent=2, default=str))
    return result.get("returncode", 0)


if __name__ == "__main__":
    raise SystemExit(main())
