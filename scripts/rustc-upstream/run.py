#!/usr/bin/env python3
"""Plan preserved rustc tests and run the official compiletest in two arms."""

import argparse
from collections import Counter, defaultdict
from dataclasses import asdict
import json
from pathlib import Path
import re
import subprocess
import sys

from checks import compare_runs, object_provenance, sha256
from protocol import auxiliary_profile_blocks, parse_test

REPOSITORY = Path(__file__).resolve().parents[2]
DEFAULT_MANIFEST = REPOSITORY / "crates/rustc_codegen_kfrc/tests/rustc-upstream/manifest.json"
PROFILE_FLAGS = ("-Cpanic=abort", "-Cprefer-dynamic=no", "-Csave-temps=yes", "-Ccodegen-units=1", "-Cdebuginfo=0", "-Zmir-opt-level=0")


def write_json(path, value):
    Path(path).write_text(json.dumps(value, indent=2, default=str) + "\n")


def contained(root, name):
    path = (root / name).resolve()
    if not path.is_relative_to(root):
        raise ValueError(f"manifest path escapes source root: {name}")
    return path


def verify_manifest(path):
    manifest = json.loads(path.read_text())
    if manifest["schema_version"] != 1:
        raise ValueError("unsupported manifest schema")
    root = (path.parent / manifest["source_root"]).resolve()
    for record in manifest["files"]:
        source = contained(root, record["source"])
        if sha256(source) != record["sha256"]:
            raise ValueError(f"upstream source hash mismatch: {record['source']}")
    for record in manifest["upstream"]["licenses"]:
        if sha256(contained(path.parent, record["source"])) != record["sha256"]:
            raise ValueError(f"upstream license hash mismatch: {record['source']}")
    for record in manifest.get("support_files", []):
        if sha256(contained(path.parent, record["source"])) != record["sha256"]:
            raise ValueError(f"upstream executor support hash mismatch: {record['source']}")
    for record in manifest["tests"]:
        if sha256(contained(root, record["source"])) != record["source_sha256"]:
            raise ValueError(f"test source hash mismatch: {record['source']}")
    return manifest, root


def profile_blocks(variants, root):
    reasons = []
    for variant in variants:
        reasons.extend(variant["blocked"])
        reasons.extend(auxiliary_profile_blocks(variant, root))
        if variant["mode"] not in ("run-pass", "build-pass", "run-fail"):
            reasons.append(f"backend batch does not execute mode {variant['mode']}")
        # Linux supports unwinding, so upstream run-fail rejects crashes.
        # The current backend requires panic=abort. Preserve the upstream
        # contract by reporting this incompatibility instead of accepting -6.
        if variant["mode"] == "run-fail":
            reasons.append("abort-only PoC profile cannot preserve upstream run-fail's non-crash contract")
        flags = " ".join(variant["compile_flags"])
        if re.search(r"(?:-C\s*panic=unwind|--test\b|-Z\s*no-codegen\b)", flags):
            reasons.append("source flags require unsupported unwind/test/no-codegen profile")
        if re.search(r"(?:--emit(?:=|\s)|--out-dir(?:=|\s)|(?:^|\s)-o(?:\s|$)|(?:-Z\s*|--)codegen-backend)", flags):
            reasons.append("source controls output/backend flags required by provenance capture")
        for auxiliary in variant["auxiliary"]:
            if auxiliary["kind"] in ("proc-macro", "aux-codegen-backend"):
                reasons.append(f"unsupported auxiliary kind {auxiliary['kind']}")
        if any(directive["name"] in ("ignore-backends", "needs-backends") for directive in variant["directives"]):
            reasons.append("upstream backend capability enum cannot represent kfrc")
    return list(dict.fromkeys(reasons))


def plan(manifest, root, target, families, limit):
    cases, admitted = [], 0
    for test in manifest["tests"]:
        case = {"id": test["id"], "source": test["source"], "family": test["family"],
                "selection": test["selection"], "status": test["selection"]["status"].upper()}
        if test["selection"]["status"] == "selected":
            case["variants"] = parse_test(root / test["source"], root, target, host=target)
            case["blocked"] = profile_blocks(case["variants"], root)
            if case["blocked"]:
                case["status"] = "BLOCKED"
            elif all(v["status"] == "skipped" for v in case["variants"]):
                case["status"] = "TARGET_SKIP"
            elif families and test["family"] not in families or limit is not None and admitted >= limit:
                case["status"] = "NOT_REQUESTED"
            else:
                case["status"] = "READY"
                admitted += 1
        cases.append(case)
    return cases


def test_events(result):
    return [event for event in result.get("events", []) if event.get("type") == "test"]


def terminal_events(result):
    return {event["name"]: event["event"] for event in test_events(result)
            if event.get("event") in ("ok", "failed", "ignored")}


def run_case(upstream, args, context, root, case, output):
    record = dict(case)
    case_dir = output / "cases" / case["id"]
    case_dir.mkdir(parents=True)
    arms = {}
    for arm in ("llvm", "kfrc"):
        logs, build = case_dir / arm / "logs", case_dir / arm / "build"
        logs.mkdir(parents=True)
        build.mkdir()
        result = upstream.invoke(
            args.compiletest.resolve(), context, root.parent, build, case["source"],
            backend=args.backend.resolve() if arm == "kfrc" else None,
            rustc_flags=PROFILE_FLAGS, runner=Path(__file__).with_name("capture-run.py"),
            logs=logs, env_overrides={"KFRC_UPSTREAM_CAPTURE_ROOT": str(logs / "runs"),
                                      "KFRC_UPSTREAM_BUILD_ROOT": str(build), "RUST_BACKTRACE": "0"},
        )
        arms[arm] = result
        if result["returncode"] != 0:
            status = ("LLVM_FAIL" if arm == "llvm" else "KFRC_FAIL") if terminal_events(result) else "HARNESS_FAIL"
            record.update(status=status, arms=arms)
            write_json(case_dir / "result.json", record)
            return record
        events = terminal_events(result)
        if not events:
            record.update(status="HARNESS_FAIL", reason=f"{arm}: no official test completion events", arms=arms)
            write_json(case_dir / "result.json", record)
            return record
        if all(event == "ignored" for event in events.values()):
            record.update(status="UPSTREAM_IGNORED", arms=arms)
            write_json(case_dir / "result.json", record)
            return record
    llvm, kfrc = case_dir / "llvm", case_dir / "kfrc"
    comparison = compare_runs(llvm / "logs", kfrc / "logs", llvm / "build", kfrc / "build",
                              expected_runtime=any(v["mode"].startswith("run-") for v in case["variants"]))
    provenance = object_provenance(kfrc / "build", kfrc / "logs")
    status = "PASS"
    if terminal_events(arms["llvm"]) != terminal_events(arms["kfrc"]):
        comparison["differences"].append("official completion identities/results differ")
    if comparison["differences"]:
        status = "DIFFERENCE"
    elif not provenance["ok"]:
        status = "PROVENANCE_FAIL"
    record.update(status=status, arms=arms, comparison=comparison, provenance=provenance)
    write_json(case_dir / "result.json", record)
    return record


def summary(cases):
    families = defaultdict(Counter)
    for case in cases:
        families[case["family"]][case["status"]] += 1
    counts = Counter(case["status"] for case in cases)
    return {"total_imported_candidates": len(cases), "counts": dict(counts),
            "families": {key: dict(value) for key, value in families.items()},
            "denominator_note": "PASS is a completed source-file case, including all official revisions; blocked/excluded/ignored/deferred are separate and are not passes",
            "official_variants_passed": sum(sum(value == "ok" for value in terminal_events(case.get("arms", {}).get("kfrc", {})).values()) for case in cases)}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--manifest", type=Path, default=DEFAULT_MANIFEST)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--target", default="x86_64-unknown-linux-gnu")
    parser.add_argument("--backend", type=Path)
    parser.add_argument("--compiletest", type=Path)
    parser.add_argument("--rustc", type=Path)
    parser.add_argument("--family", action="append", default=[])
    parser.add_argument("--limit", type=int)
    parser.add_argument("--plan-only", action="store_true")
    args = parser.parse_args()
    if args.out.exists():
        parser.error("--out must be a fresh directory")
    if args.limit is not None and args.limit < 1:
        parser.error("--limit must be positive")
    manifest, root = verify_manifest(args.manifest.resolve())
    cases = plan(manifest, root, args.target, set(args.family), args.limit)
    output = args.out.resolve()
    output.mkdir(parents=True)
    write_json(output / "plan.json", cases)
    write_json(output / "inputs.json", {"manifest": str(args.manifest.resolve()), "manifest_sha256": sha256(args.manifest),
               "upstream": manifest["upstream"], "target": args.target, "profile_flags": PROFILE_FLAGS,
               "adapter_files": {path.name: sha256(path) for path in Path(__file__).parent.glob("*.py")}})
    if args.plan_only:
        report = summary(cases)
        write_json(output / "summary.json", report)
        print(json.dumps(report))
        return
    if not args.backend or not args.compiletest:
        parser.error("execution requires --backend and --compiletest")
    import compiletest as upstream
    if args.rustc is None:
        args.rustc = Path(subprocess.check_output(["rustup", "which", "--toolchain", manifest["upstream"]["version"], "rustc"], text=True).strip())
    context = upstream.probe_toolchain(args.rustc.resolve())
    if context.target != args.target or context.host != args.target:
        parser.error("first batch requires native host == target; cross execution is not configured")
    write_json(output / "toolchain.json", asdict(context))
    write_json(output / "binaries.json", {"backend": sha256(args.backend), "compiletest": sha256(args.compiletest)})
    stopped = {}
    harness_blocker = None
    with (output / "results.jsonl").open("w") as events:
        for index, case in enumerate(cases):
            if case["status"] == "READY":
                if harness_blocker is not None:
                    case.update(status="DEFERRED_HARNESS", blocked_by=harness_blocker)
                elif case["family"] in stopped:
                    case.update(status="DEFERRED_FAMILY", blocked_by=stopped[case["family"]])
                else:
                    try:
                        case = run_case(upstream, args, context, root, case, output)
                    except Exception as error:
                        case = dict(case, status="HARNESS_FAIL", reason=f"{type(error).__name__}: {error}")
                    if case["status"] not in ("PASS", "UPSTREAM_IGNORED"):
                        stopped[case["family"]] = case["id"]
                    if case["status"] == "HARNESS_FAIL":
                        harness_blocker = case["id"]
                    print(f"{case['status']} {case['id']}", flush=True)
                cases[index] = case
            events.write(json.dumps(case) + "\n")
            events.flush()
            write_json(output / "summary.json", summary(cases))
    # Recheck original bytes after execution as well as before compilation.
    verify_manifest(args.manifest.resolve())
    report = summary(cases)
    write_json(output / "summary.json", report)
    print(json.dumps(report), flush=True)
    if any(case["status"] in ("LLVM_FAIL", "KFRC_FAIL", "DIFFERENCE", "PROVENANCE_FAIL", "HARNESS_FAIL") for case in cases):
        raise SystemExit(1)


if __name__ == "__main__":
    main()
