#!/usr/bin/env python3
"""Compare only observed test identities and outcomes, retaining raw JSON logs."""
import json
import pathlib
import sys

work = pathlib.Path(sys.argv[1])
suites = {"test_autotrait": 1, "test_identifier": 3, "test_version": 10, "test_version_req": 20}
results = {}
for arm in ["llvm", "kfrc"]:
    tests = {}
    for suite, expected in suites.items():
        events = [json.loads(line) for line in (work / arm / f"{suite}.jsonl").read_text().splitlines()]
        outcomes = {
            row["name"]: row["event"] for row in events
            if row.get("type") == "test" and row.get("event") in {"ok", "failed", "ignored"}
        }
        if len(outcomes) != expected or any(event != "ok" for event in outcomes.values()):
            raise SystemExit(f"{arm}/{suite}: expected {expected} passing tests, observed {outcomes}")
        if len([e for e in events if e.get("type") == "suite" and e.get("event") == "ok"]) != 1:
            raise SystemExit(f"{arm}/{suite}: missing successful suite completion")
        tests[suite] = outcomes
    results[arm] = tests
if results["kfrc"] != results["llvm"]:
    raise SystemExit("LLVM and kfrc test identities/outcomes differ")
summary = {"crate": "semver", "version": "1.0.28", "features": ["std"],
    "serde_enabled": False, "upstream_tests_per_arm": sum(suites.values()),
    "scope": "unchanged upstream integration tests; LLVM libtest harness links the kfrc semver library",
    "outcomes": results,
    "debug_provenance": {arm: json.loads((work / arm / "debug-provenance.json").read_text()) for arm in results}}
(work / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")
print(f"PASS: semver 1.0.28, {sum(suites.values())} unchanged upstream integration tests per arm")
