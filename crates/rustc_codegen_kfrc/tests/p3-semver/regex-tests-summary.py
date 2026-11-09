#!/usr/bin/env python3
"""Require every test discovered by stock rustc to pass under both backends."""
import json
import pathlib
import sys

work = pathlib.Path(sys.argv[1])
results = {}
lists = {}
for arm in ["llvm", "kfrc"]:
    names = [line[:-6] for line in (work / arm / "tests.list").read_text().splitlines()
             if line.endswith(": test")]
    if not names or len(names) != len(set(names)):
        raise SystemExit(f"{arm}: missing tests or duplicate test identities")
    lists[arm] = sorted(names)
    events = [json.loads(line) for line in (work / arm / "tests.jsonl").read_text().splitlines()]
    outcomes = {}
    for row in events:
        if row.get("type") == "test" and row.get("event") in {"ok", "failed", "ignored"}:
            if row["name"] in outcomes:
                raise SystemExit(f"{arm}: duplicate completion for {row['name']}")
            outcomes[row["name"]] = row["event"]
    if sorted(outcomes) != sorted(names):
        raise SystemExit(f"{arm}: executed tests differ from discovered tests")
    if any(outcome != "ok" for outcome in outcomes.values()):
        raise SystemExit(f"{arm}: failed or ignored tests: {outcomes}")
    suites = [row for row in events if row.get("type") == "suite" and row.get("event") == "ok"]
    if len(suites) != 1 or suites[0].get("passed") != len(names):
        raise SystemExit(f"{arm}: missing or incomplete suite completion")
    results[arm] = outcomes
if lists["llvm"] != lists["kfrc"] or results["llvm"] != results["kfrc"]:
    raise SystemExit("LLVM and kfrc did not discover and pass identical upstream tests")
summary = {"crate": "regex-syntax", "version": "0.8.8",
    "features": json.loads((work / "test-features.json").read_text())["enabled"],
    "tests_per_arm": len(lists["llvm"]),
    "scope": "unchanged upstream internal unit tests, generated test harness and library code compiled by selected backend; libtest/std prebuilt by LLVM",
    "outcomes": results}
(work / "tests-summary.json").write_text(json.dumps(summary, indent=2) + "\n")
print(f"PASS: regex-syntax 0.8.8 all {len(lists['llvm'])} unchanged upstream internal tests per backend")
