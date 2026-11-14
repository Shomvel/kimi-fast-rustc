#!/usr/bin/env python3
"""Verify unchanged source scope and the linked Version Debug object owner."""

import hashlib
import json
from pathlib import Path
import re
import sys
import tomllib


SUITES = ["test_autotrait", "test_identifier", "test_version", "test_version_req"]


def verify_source(work: Path) -> dict:
    receipt = json.loads((work / "source.json").read_text())
    source = work / f"semver-{receipt['version']}"
    actual = {
        str(path.relative_to(source)): hashlib.sha256(path.read_bytes()).hexdigest()
        for path in sorted(source.rglob("*")) if path.is_file()
    }
    if actual != receipt["source_files_sha256"]:
        raise SystemExit("semver published source files changed after extraction")
    manifest = tomllib.loads((source / "Cargo.toml").read_text())
    expected = [{"name": suite, "path": f"tests/{suite}.rs"} for suite in SUITES]
    if manifest["package"].get("autotests") is not False or manifest.get("test") != expected:
        raise SystemExit("semver published integration suite manifest differs from the runner")
    primary = sorted(str(path.relative_to(source)) for path in (source / "tests").glob("*.rs"))
    if primary != sorted(item["path"] for item in expected):
        raise SystemExit("semver contains an unaccounted integration test file")
    source_tests = [
        str(path.relative_to(source)) for path in (source / "src").rglob("*.rs")
        if re.search(r"#\s*\[\s*test\s*\]", path.read_text())
    ]
    if source_tests:
        raise SystemExit(f"semver contains unaccounted source unit tests: {source_tests}")
    return {"verified_source_files": len(actual), "upstream_integration_suites": expected, "source_unit_tests": []}


def object_tables(text: str, single_object: str | None = None) -> list[dict]:
    tables = []
    current = None
    if single_object is not None and not any(line.startswith("File: ") for line in text.splitlines()):
        current = {"object": single_object, "writer": [], "functions": []}
        tables.append(current)
    for line in text.splitlines():
        if line.startswith("File: "):
            current = {"object": line.removeprefix("File: "), "writer": [], "functions": []}
            tables.append(current)
        if current is None:
            continue
        fields = line.split()
        if len(fields) < 8 or not fields[0].endswith(":"):
            continue
        if fields[3] == "FILE":
            current["writer"].append(fields[7])
        elif fields[3] == "FUNC" and fields[6] != "UND":
            current["functions"].append(fields[7])
    return tables


def version_debug_symbol(symbol: str) -> bool:
    # Both names are visible in the pinned toolchain's v0 symbols. VersionReq
    # is deliberately not accepted as proof of the non-inline Version impl.
    return "6semver" in symbol and "7Version" in symbol and "5Debug3fmt" in symbol


def verify_objects(project: Path, arm: str) -> dict:
    objects = list((project / "objects").glob("*.o"))
    definitions = [
        {"object": table["object"], "writer": table["writer"], "symbol": symbol}
        for table in object_tables((project / "object-symbols.txt").read_text(), str(objects[0]) if len(objects) == 1 else None)
        for symbol in table["functions"] if version_debug_symbol(symbol)
    ]
    if not definitions:
        raise SystemExit(f"{arm}: Version::Debug::fmt is not defined in the semver rlib objects")
    if arm == "kfrc" and any("kfrc" not in item["writer"] for item in definitions):
        raise SystemExit("Version::Debug::fmt is not owned by a kfrc writer object")
    executable_symbols = set()
    for line in (project / "driver-symbols.txt").read_text().splitlines():
        fields = line.split()
        if len(fields) >= 8 and fields[3] == "FUNC" and fields[6] != "UND":
            executable_symbols.add(fields[7])
    if not all(item["symbol"] in executable_symbols for item in definitions):
        raise SystemExit(f"{arm}: driver does not retain the same Version::Debug::fmt definition")
    output = (project / "workload.out").read_text()
    if "debug=Version {" not in output or "debug=VersionReq {" not in output:
        raise SystemExit(f"{arm}: successful Version/VersionReq Debug workload is missing")
    return {
        "arm": arm,
        "version_debug_definitions": definitions,
        "same_symbol_defined_in_driver": True,
        "visible_version_debug": True,
        "version_req_debug_limitation": "derived inline method may be instantiated by the LLVM driver; it is not proof of a kfrc method definition",
    }


if __name__ == "__main__":
    phase, path, *rest = sys.argv[1:]
    if phase == "source":
        result = verify_source(Path(path))
    elif phase == "objects":
        result = verify_objects(Path(path), rest[0])
    else:
        raise SystemExit("phase must be source or objects")
    print(json.dumps(result, indent=2))
