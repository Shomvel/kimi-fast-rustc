#!/usr/bin/env python3
"""Materialize the pinned Rust behavior corpus for the shared E2E runner.

This module does not parse Rust, compile code, or evaluate a reference language.
The checked-in fixtures contain ordinary Rust compiled by the real rustc. The
runner builds the libraries with LLVM / kfrc and builds every driver with LLVM.

Usage: python3 legacy-corpus.py OUTPUT [--suite arithmetic ...]
       python3 legacy-corpus.py --check
"""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import shutil

FIXTURES = Path(__file__).resolve().parent / "fixtures" / "legacy-corpus"
SUITES = ("arithmetic", "comparison", "control", "loops", "consts", "calls", "examples")


def load_suite(name: str) -> dict:
    if name not in SUITES:
        raise ValueError(f"unknown corpus suite: {name}")
    path = FIXTURES / name
    manifest = json.loads((path / "manifest.json").read_text())
    if manifest["schema_version"] != 1:
        raise ValueError(f"unsupported manifest version in {path}")
    ids = set()
    expected_lines = []
    traps = {}
    library = (path / "lib.rs").read_text()
    driver = (path / "driver.rs").read_text()
    for case in manifest["cases"]:
        if case["id"] in ids:
            raise ValueError(f"duplicate fixture: {case['id']}")
        ids.add(case["id"])
        if hashlib.sha256(case["original_source"].encode()).hexdigest() != case["source_sha256"]:
            raise ValueError(f"source fingerprint mismatch: {case['id']}")
        if case["source"] not in library:
            raise ValueError(f"fixture source missing from lib.rs: {case['id']}")
        for index, vector in enumerate(case["vectors"]):
            identifier = f"{case['id']}:{index}"
            if len(vector["arguments"]) != len(case["argument_types"]):
                raise ValueError(f"argument count mismatch: {identifier}")
            if identifier not in driver:
                raise ValueError(f"driver invocation missing: {identifier}")
            if vector["trap"] is None:
                expected_lines.append(vector.get("stdout_before", "") + f"{identifier}={vector['expected_bits']}\n")
            else:
                traps[identifier] = vector["trap"]
    normal = [run for run in manifest["runs"] if run["expected_exit"] == "success"]
    if len(normal) != 1 or normal[0]["run_args"] or normal[0]["expected_stdout"] != "".join(expected_lines):
        raise ValueError(f"normal output contract mismatch in {name}")
    trap_runs = {run["id"]: run for run in manifest["runs"] if run["expected_exit"] == "trap"}
    if trap_runs.keys() != traps.keys():
        raise ValueError(f"trap run inventory mismatch in {name}")
    for identifier, kind in traps.items():
        run = trap_runs[identifier]
        if run["trap_kind"] != kind or run["run_args"] != [identifier]:
            raise ValueError(f"trap invocation mismatch: {identifier}")
    return manifest


def materialize(destination: Path, suites=SUITES) -> list[dict]:
    manifests = [load_suite(name) for name in suites]
    destination.mkdir(parents=True, exist_ok=True)
    for name in suites:
        target = destination / name
        target.mkdir(exist_ok=True)
        for filename in ("lib.rs", "driver.rs", "manifest.json"):
            source = FIXTURES / name / filename
            if source.resolve() != (target / filename).resolve():
                shutil.copyfile(source, target / filename)
    return manifests


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", type=Path, nargs="?")
    parser.add_argument("--suite", nargs="+", choices=SUITES)
    parser.add_argument("--check", action="store_true", help="check fixture metadata without compiling")
    args = parser.parse_args()
    selected = args.suite or SUITES
    if args.check:
        manifests = [load_suite(name) for name in selected]
    elif args.output:
        manifests = materialize(args.output, selected)
    else:
        parser.error("provide OUTPUT or --check")
    print(json.dumps({
        "suites": len(manifests),
        "cases": sum(len(m["cases"]) for m in manifests),
        "vectors": sum(len(c["vectors"]) for m in manifests for c in m["cases"]),
        "trap_runs": sum(r["expected_exit"] == "trap" for m in manifests for r in m["runs"]),
        "validation": "metadata only; real compiler results belong to legacy-corpus-run.py",
    }, indent=2))


if __name__ == "__main__":
    main()
