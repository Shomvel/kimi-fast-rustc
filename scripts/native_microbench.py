#!/usr/bin/env python3
import argparse
import hashlib
import json
import os
import platform
import shutil
import statistics
import subprocess
import tempfile
import time
from pathlib import Path


def source(index):
    return f"""fn first(value: i64) -> i64 {{
    return value + {index + 6};
}}

fn second(value: i64) -> i64 {{
    if value == 0 {{
        return 0;
    }}
    return value + second(value - 1);
}}

fn third(value: i64) -> i64 {{
    let mut result = value;
    let mut count = 0;
    while count < 3 {{
        result += 1;
        count += 1;
    }}
    return result;
}}

fn main() -> i64 {{
    return first(30) + second(3) + third(2) - {index + 5};
}}
"""


def digest(path):
    hasher = hashlib.sha256()
    with path.open("rb") as file:
        for block in iter(lambda: file.read(1024 * 1024), b""):
            hasher.update(block)
    return hasher.hexdigest()


def run(command, env=None):
    environment = os.environ.copy()
    if env:
        environment.update(env)
    result = subprocess.run(command, capture_output=True, text=True, env=environment)
    if result.returncode != 0:
        raise RuntimeError(
            f"command failed ({result.returncode}): {' '.join(map(str, command))}\n"
            f"stdout: {result.stdout}\nstderr: {result.stderr}"
        )


def main():
    parser = argparse.ArgumentParser(
        description="Measure deterministic multi-function native compilation."
    )
    parser.add_argument("--compiler", required=True, type=Path)
    parser.add_argument("--linker", default="gcc")
    parser.add_argument(
        "--mode",
        choices=("native", "batch-dag", "batch-pipeline"),
        default="native",
    )
    parser.add_argument("--jobs", default="1,2,4")
    parser.add_argument("--cache", choices=("off", "cold", "warm"), default="off")
    parser.add_argument("--samples", type=int, default=7)
    parser.add_argument("--inputs", type=int, default=8)
    parser.add_argument("--workdir", type=Path)
    parser.add_argument("--keep", action="store_true")
    parser.add_argument("--skip-run", action="store_true")
    args = parser.parse_args()
    if args.samples < 1 or args.inputs < 1:
        parser.error("samples and inputs must be positive")
    jobs = [int(value) for value in args.jobs.split(",")]
    if not jobs or any(value < 1 for value in jobs):
        parser.error("jobs must be positive integers")

    compiler = args.compiler.resolve()
    if not compiler.is_file():
        parser.error(f"compiler does not exist: {compiler}")
    temporary = args.workdir is None
    workdir = Path(tempfile.mkdtemp(prefix="kfrc-native-microbench-")) if temporary else args.workdir
    workdir.mkdir(parents=True, exist_ok=True)
    try:
        inputs = []
        workload = hashlib.sha256()
        for index in range(args.inputs):
            path = workdir / f"input-{index}.kfr"
            text = source(index)
            path.write_text(text)
            workload.update(text.encode())
            inputs.append(path)

        measurements = {}
        for job_count in jobs:
            output = workdir / f"objects-{job_count}"
            times = []
            hashes = []
            for sample in range(args.samples + 1):
                if output.exists():
                    shutil.rmtree(output)
                output.mkdir()
                cache_environment = None
                if args.cache != "off":
                    cache_dir = workdir / f"cache-{job_count}"
                    if args.cache == "cold":
                        cache_dir = workdir / f"cache-{job_count}-{sample}"
                        if cache_dir.exists():
                            shutil.rmtree(cache_dir)
                    cache_dir.mkdir(parents=True, exist_ok=True)
                    cache_environment = {"KFRC_NATIVE_CACHE": str(cache_dir)}
                started = time.perf_counter_ns()
                if args.mode == "native":
                    for path in inputs:
                        run([
                            str(compiler),
                            "native",
                            str(path),
                            "--jobs",
                            str(job_count),
                            f"--emit=obj={output / (path.stem + '.o')}",
                        ], env=cache_environment)
                else:
                    command = [
                        str(compiler),
                        "native-batch",
                        "--jobs",
                        str(job_count),
                        "--out-dir",
                        str(output),
                    ]
                    if args.mode == "batch-pipeline":
                        command.append("--pipeline")
                    command.extend(map(str, inputs))
                    run(command, env=cache_environment)
                elapsed = time.perf_counter_ns() - started
                if sample:
                    times.append(elapsed / 1_000_000)
                current_hashes = [digest(output / (path.stem + ".o")) for path in inputs]
                if hashes and current_hashes != hashes:
                    raise RuntimeError(f"objects are not deterministic at jobs={job_count}")
                hashes = current_hashes
            measurements[job_count] = {
                "median_ms": statistics.median(times),
                "min_ms": min(times),
                "max_ms": max(times),
                "samples_ms": times,
                "object_sha256": hashes,
                "object_bytes": [(output / (path.stem + ".o")).stat().st_size for path in inputs],
            }

        reference = measurements[jobs[0]]["object_sha256"]
        for job_count in jobs[1:]:
            if measurements[job_count]["object_sha256"] != reference:
                raise RuntimeError(f"objects differ between jobs 1 and {job_count}")

        executions = []
        if not args.skip_run:
            output = workdir / f"objects-{jobs[0]}"
            for path in inputs:
                executable = output / path.stem
                run([args.linker, str(output / (path.stem + ".o")), "-o", str(executable)])
                result = subprocess.run([str(executable)], capture_output=True)
                if result.returncode != 42:
                    raise RuntimeError(f"{path.name} returned {result.returncode}, expected 42")
                executions.append({"input": path.name, "exit": result.returncode})

        print(json.dumps({
            "compiler": str(compiler),
            "compiler_sha256": digest(compiler),
            "mode": args.mode,
            "cache_mode": args.cache,
            "workload_sha256": workload.hexdigest(),
            "inputs": args.inputs,
            "functions_per_input": 4,
            "samples": args.samples,
            "warmups": 1,
            "host": {
                "system": platform.system(),
                "machine": platform.machine(),
                "processor": platform.processor(),
                "cpu_count": os.cpu_count(),
            },
            "measurements": measurements,
            "executions": executions,
            "workdir": str(workdir),
        }, indent=2, sort_keys=True))
    finally:
        if temporary and not args.keep:
            shutil.rmtree(workdir)


if __name__ == "__main__":
    main()
