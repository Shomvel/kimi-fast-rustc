#!/usr/bin/env python3
"""Import byte-exact rustc UI tests from one checksummed upstream archive.

Selection is an initial capability/scope ledger, never an execution result.
The runner must parse the preserved compiletest protocol before each revision.
"""

from __future__ import annotations

import argparse
import collections
import hashlib
import json
from pathlib import Path, PurePosixPath
import re
import tarfile
import urllib.request


COMMIT = "e408947bfd200af42db322daf0fadfe7e26d3bd1"
ARCHIVE_URL = f"https://codeload.github.com/rust-lang/rust/tar.gz/{COMMIT}"
ARCHIVE_SHA256 = "063f7b364a2f0dee6d8f6733486ea7b6510c79682129448b2e6bd4ffc3e2a8fe"
RAW_ROOT = f"https://raw.githubusercontent.com/rust-lang/rust/{COMMIT}/"
ARCHIVE_ROOT = f"rust-{COMMIT}/"
REPOSITORY = Path(__file__).resolve().parents[2]
DEFAULT_OUTPUT = REPOSITORY / "crates/rustc_codegen_kfrc/tests/rustc-upstream"
REQUIRED_SUPPORT_FILES = ["tests/auxiliary/minicore.rs"]
REQUIRED_RESOURCE_DIRECTORIES = [
    "src/etc/natvis",
    "src/tools/run-make-support",
    "src/tools/compiletest",
]
REQUIRED_RESOURCE_FILES = [
    "src/etc/rust_types.py",
    "src/etc/gdb_load_rust_pretty_printers.py",
    "src/etc/gdb_lookup.py",
    "src/etc/gdb_providers.py",
    "src/etc/lldb_batchmode.py",
    "src/etc/lldb_lookup.py",
    "src/etc/lldb_providers.py",
]

# The complete pool is every explicit run-pass/build-pass/run-fail primary .rs
# under these directories. Compile-fail/check-pass-only tests are outside it.
FAMILIES = {
    "integer": ["binop", "numbers-arithmetic", "numeric"],
    "control-flow": ["expr", "loops", "for-loop-while", "return"],
    "reference": ["borrowck", "ptr_ops", "raw-ref-op", "deref"],
    "aggregate": ["tuple", "destructuring-assignment", "structs"],
    "enum": ["enum", "enum-discriminant"],
    "call": ["fn", "function-pointer"],
    "drop": ["drop", "moves"],
    "panic": ["panics"],
}
QUOTAS = {
    "integer": 25,
    "control-flow": 25,
    "reference": 25,
    "aggregate": 20,
    "enum": 20,
    "call": 10,
    "drop": 30,
    "panic": 5,
}
DIRECTIVE = re.compile(
    r"^\s*//@\s*(?:\[([^]]+)\]\s*)?([\w-]+)(?::\s*(.*))?$"
)
MODES = {"run-pass", "build-pass", "run-fail"}
CLASSIFIER_VERSION = 3
SELECTION_BATCH_ID = "rustc-1.94.1-batch-01"
BASELINE_MANIFEST_SHA256 = "ff8d0158fdb388cf5a2e55f01e391514c2ae2d0c19dcd0e3398bc9de4b35f6a4"
BASELINE_MEMBERSHIP_SHA256 = "2b92df7211a4999dafa46a447505af39edb00b21c3af8fffe66293506ddb7f38"
CLASSIFIER_V2_MANIFEST_SHA256 = "b816a0cd6d90c5bfc3788c3a8724623c5382b3f723502d0cbf55ca206a7934d6"
NUMBER_TOKEN = re.compile(
    r"(?<!\w)(?:"
    r"0[xX][0-9a-fA-F_]+[a-zA-Z0-9_]*|"
    r"0[oO][0-7_]+[a-zA-Z0-9_]*|"
    r"0[bB][01_]+[a-zA-Z0-9_]*|"
    r"[0-9][0-9_]*(?:\.[0-9_]+)?(?:[eE][+-]?[0-9_]+)?[a-zA-Z0-9_]*"
    r")"
)


def sha(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def directives(text: str) -> list[dict]:
    found = []
    for number, line in enumerate(text.splitlines(), 1):
        match = DIRECTIVE.match(line)
        if match:
            revision, name, value = match.groups()
            found.append({
                "line": number,
                "text": line,
                "revision": revision,
                "name": name,
                "value": value or "",
            })
    return found


def float_suffix_literals(code: str) -> list[str]:
    """Find decimal float suffixes while consuming radix integers as one token.

    `3f32`, `3_f32` and `1e6f64` need float support. `0xff32` and
    `0xdead_beef_f64` are hexadecimal integers whose digits must not be
    mistaken for decimal suffixes. Invalid non-decimal float suffixes belong
    to frontend diagnostics, not this executable-test capability classifier.
    """
    return [
        token
        for match in NUMBER_TOKEN.finditer(code)
        if not (token := match.group()).lower().startswith(("0x", "0o", "0b"))
        and re.search(r"(?:f16|f32|f64|f128)$", token)
    ]


def wide_integer_literals(code: str) -> list[str]:
    """Find signed/unsigned 128-bit suffixes on decimal or radix literals."""
    return [
        match.group()
        for match in NUMBER_TOKEN.finditer(code)
        if re.search(r"(?:i128|u128)$", match.group())
    ]


def initial_blocks(text: str, protocol: list[dict], *, classifier_version: int = CLASSIFIER_VERSION) -> list[str]:
    # Conservative lexical capability checks. False positives remain visible in
    # the ledger and can be reclassified without modifying upstream sources.
    code = re.sub(r"/\*.*?\*/|//[^\n]*", "", text, flags=re.S)
    checks = [
        (r"\b(?:f16|f32|f64|f128)\b|\b\d+\.\d+(?:[eE][+-]?\d+)?", "floating-point"),
        (r"\b(?:i128|u128)\b", "integer-wider-than-64-bits"),
        (r"\b(?:catch_unwind|resume_unwind|AssertUnwindSafe)\b", "panic-unwind"),
        (r"\b(?:Atomic\w*|Ordering)\b|\b(?:atomic|atomics)::", "atomics"),
        (r"\b(?:simd\w*|Simd|__m128\w*|__m256\w*|__m512\w*)\b", "simd"),
        (r"\b(?:asm|global_asm|naked_asm)!", "inline-assembly"),
        (r"\b(?:thread|thread_local)::|\bthread_local!|\bthread::spawn", "threads-or-tls"),
        (r"\b(?:async|await|yield)\b|\b(?:Future|Coroutine)\b", "async-or-coroutine"),
        (r"#!\s*\[\s*(?:no_std|no_core|no_main)\b", "custom-runtime-or-core"),
        (r"\bextern\s+\"(?:C|C-unwind|system|stdcall|vectorcall)\"", "foreign-abi-or-runtime"),
        (r"\b(?:Command|SystemTime|Instant|RandomState)\b|\bstd::env::(?:args|var)\b", "external-input-or-process"),
        (r"\{(?::[^}]*)?p\}|\b(?:getpid|process::id)\b", "nondeterministic-output"),
        (r"#\s*\[\s*(?:rustc_|lang\s*=|start\b)", "compiler-internal-or-custom-entry"),
    ]
    reasons = [f"capability:{name}" for pattern, name in checks if re.search(pattern, code)]
    if classifier_version >= 2 and float_suffix_literals(code):
        reasons.append("capability:floating-point")
    if classifier_version >= 3 and wide_integer_literals(code):
        reasons.append("capability:integer-wider-than-64-bits")
    for directive in protocol:
        name, value = directive["name"], directive["value"]
        if name == "needs-unwind" or (name == "compile-flags" and re.search(r"panic[= ]unwind", value)):
            reasons.append("capability:panic-unwind")
        if name in {"needs-asm-support", "needs-sanitizer-address", "needs-threads"}:
            reasons.append(f"capability:{name}")
        if name == "compile-flags" and re.search(r"(?:-Clto|-C\s*lto|--test|--target\b|--crate-type\s*=?\s*(?:dylib|cdylib|staticlib))", value):
            reasons.append("harness:nonstandard-build-or-target")
        if name in {"run-rustfix", "run-rustfix-only-machine-applicable"}:
            reasons.append("harness:rustfix")
    return sorted(set(reasons))


class Sources:
    def __init__(self, archive: tarfile.TarFile):
        self.archive = archive
        self.members = {
            member.name.removeprefix(ARCHIVE_ROOT): member
            for member in archive.getmembers()
            if member.isfile() and member.name.startswith(ARCHIVE_ROOT)
        }
        self.cache: dict[str, bytes] = {}

    def read(self, path: str) -> bytes:
        if path not in self.cache:
            stream = self.archive.extractfile(self.members[path])
            assert stream is not None
            self.cache[path] = stream.read()
        return self.cache[path]

    def descriptor(self, path: str) -> dict:
        descriptor = {
            "source": path.removeprefix("tests/"),
            "sha256": sha(self.read(path)),
            "upstream_url": RAW_ROOT + path,
        }
        if path.endswith(".rs"):
            descriptor["directives"] = directives(self.read(path).decode("utf-8"))
        return descriptor

    def required_resources(self) -> set[str]:
        resources = set(REQUIRED_RESOURCE_FILES)
        for directory in REQUIRED_RESOURCE_DIRECTORIES:
            members = {path for path in self.members if path.startswith(directory + "/")}
            if not members:
                raise SystemExit(f"pinned archive omits required resource directory: {directory}")
            resources.update(members)
        return resources

    def dependencies(self, path: str, protocol: list[dict]) -> set[str]:
        parent = PurePosixPath(path).parent
        auxiliary = str(parent / "auxiliary") + "/"
        dependencies: set[str] = set()
        if any(d["name"] in {"aux-build", "aux-crate", "aux-bin"} for d in protocol):
            # Copy the whole adjacent auxiliary tree, preserving dependencies,
            # nested modules, files loaded by macros, and its own directives.
            dependencies.update(p for p in self.members if p.startswith(auxiliary))
        stem = str(PurePosixPath(path).with_suffix(""))
        dependencies.update(p for p in self.members if p.startswith(stem + ".") and p != path)
        pending = [path, *dependencies]
        seen = set()
        while pending:
            current = pending.pop()
            if current in seen or not current.endswith(".rs"):
                continue
            seen.add(current)
            text = self.read(current).decode("utf-8")
            base = PurePosixPath(current).parent
            literal_paths = re.findall(r'\binclude(?:_str|_bytes)?!\s*\(\s*"([^"\n]+)"', text)
            literal_paths += re.findall(r'#\s*\[\s*path\s*=\s*"([^"\n]+)"\s*\]', text)
            for module in re.findall(r"\bmod\s+(\w+)\s*;", text):
                literal_paths.extend([f"{module}.rs", f"{module}/mod.rs"])
                literal_paths.extend([f"{PurePosixPath(current).stem}/{module}.rs", f"{PurePosixPath(current).stem}/{module}/mod.rs"])
            for relative in literal_paths:
                # Resolve .. without ever allowing the archive to write paths.
                parts = []
                for part in (base / relative).parts:
                    if part == "..":
                        if parts:
                            parts.pop()
                    elif part != ".":
                        parts.append(part)
                other = "/".join(parts)
                if other.startswith("tests/") and other in self.members and other != path:
                    if other not in dependencies:
                        dependencies.add(other)
                        pending.append(other)
        return dependencies


def classified_selection(sources: Sources, test: dict, version: int) -> dict:
    reasons = initial_blocks(sources.read("tests/" + test["source"]).decode("utf-8"), test["directives"], classifier_version=version)
    for dependency in test["dependencies"]:
        path = dependency["source"]
        if path.endswith(".rs"):
            text = sources.read("tests/" + path).decode("utf-8")
            reasons += [f"dependency:{path}:{reason}" for reason in initial_blocks(text, directives(text), classifier_version=version)]
    return {"status": "blocked", "reasons": sorted(set(reasons))} if reasons else dict(test["initial_selection"])


def classification_updates(sources: Sources, tests: list[dict]) -> list[dict]:
    updates = []
    for version in range(2, CLASSIFIER_VERSION + 1):
        for test in tests:
            previous = classified_selection(sources, test, version - 1)
            current = classified_selection(sources, test, version)
            if previous == current:
                continue
            entry = {
                "id": test["id"],
                "classifier_version": version,
                "initial_selection": test["initial_selection"],
                "selection": current,
            }
            text = sources.read("tests/" + test["source"]).decode("utf-8")
            if version == 2:
                # Preserve the published version-2 delta exactly; later
                # corrections append entries instead of relabeling history.
                entry["reason"] = "recognize decimal integer/exponent literals with float suffixes while preserving radix integer tokens"
                entry["float_suffix_evidence"] = [
                    {"line": number, "tokens": float_suffix_literals(line)}
                    for number, line in enumerate(text.splitlines(), 1)
                    if float_suffix_literals(line) and not line.lstrip().startswith("//")
                ]
            else:
                entry["previous_selection"] = previous
                entry["reason"] = "recognize i128/u128 suffixes on decimal, hexadecimal, octal and binary integer literals, including macro arguments"
                entry["wide_integer_evidence"] = [
                    {"line": number, "tokens": wide_integer_literals(line)}
                    for number, line in enumerate(text.splitlines(), 1)
                    if wide_integer_literals(line) and not line.lstrip().startswith("//")
                ]
            updates.append(entry)
    return updates


def import_sources(archive_path: Path, output: Path) -> dict:
    if sha(archive_path.read_bytes()) != ARCHIVE_SHA256:
        raise SystemExit("archive SHA-256 differs from the pinned upstream archive")
    output.mkdir(parents=True, exist_ok=True)
    manifest_path = output / "manifest.json"
    if manifest_path.exists():
        raise SystemExit(f"refusing to overwrite {manifest_path}")
    vendor = output / "vendor"
    tests = []
    copied: set[str] = set(REQUIRED_SUPPORT_FILES)
    with tarfile.open(archive_path, "r:gz") as archive:
        sources = Sources(archive)
        directory_family = {directory: family for family, directories in FAMILIES.items() for directory in directories}
        for path in sorted(sources.members):
            parts = PurePosixPath(path).parts
            if len(parts) < 4 or parts[:2] != ("tests", "ui") or parts[2] not in directory_family:
                continue
            if not path.endswith(".rs") or "auxiliary" in parts:
                continue
            text = sources.read(path).decode("utf-8")
            protocol = directives(text)
            modes = sorted({d["name"] for d in protocol if d["name"] in MODES})
            if not modes:
                continue
            family = directory_family[parts[2]]
            baseline_reasons = initial_blocks(text, protocol, classifier_version=1)
            reasons = initial_blocks(text, protocol)
            dependencies = sources.dependencies(path, protocol)
            for dep in dependencies:
                if dep.endswith(".rs"):
                    dep_text = sources.read(dep).decode("utf-8")
                    baseline_reasons += [f"dependency:{dep.removeprefix('tests/')}:{reason}" for reason in initial_blocks(dep_text, directives(dep_text), classifier_version=1)]
                    reasons += [f"dependency:{dep.removeprefix('tests/')}:{reason}" for reason in initial_blocks(dep_text, directives(dep_text))]
            tests.append({
                "id": path.removeprefix("tests/").removesuffix(".rs"),
                "suite": "ui",
                "family": family,
                "source": path.removeprefix("tests/"),
                "source_sha256": sha(sources.read(path)),
                "upstream_url": RAW_ROOT + path,
                "mode": modes[0] if len(modes) == 1 else "revision-dependent",
                "directives": protocol,
                "auxiliaries": [sources.descriptor(dep) for dep in sorted(dependencies) if "/auxiliary/" in dep],
                "dependencies": [sources.descriptor(dep) for dep in sorted(dependencies)],
                "initial_selection": {"status": "blocked" if baseline_reasons else "pending", "reasons": sorted(set(baseline_reasons))},
                "selection": {"status": "blocked" if reasons else "pending", "reasons": sorted(set(reasons))},
            })
            copied.update({path, *dependencies})
        # Reproduce batch 01 membership with its original classifier. Later
        # capability corrections can block members but never refill a quota.
        for family, quota in QUOTAS.items():
            eligible = [test for test in tests if test["family"] == family and test["initial_selection"]["status"] == "pending"]
            def priority(test: dict) -> tuple:
                data = sources.read("tests/" + test["source"])
                return (not bool(re.search(rb"\bassert(?:_eq|_ne)?!", data)), len(data), test["id"])
            for position, test in enumerate(sorted(eligible, key=priority)):
                selected = position < quota
                test["initial_selection"] = {
                    "status": "selected" if selected else "excluded",
                    "reasons": [f"batch-one:{family}:concise-assertion-priority" if selected else f"batch-one:{family}:quota-{quota}-deferred"],
                }
        for test in tests:
            if test["selection"]["status"] == "pending":
                test["selection"] = dict(test["initial_selection"])
        original_members = [test["id"] for test in tests if test["initial_selection"]["status"] == "selected"]
        membership_sha256 = sha(("\n".join(original_members) + "\n").encode())
        if membership_sha256 != BASELINE_MEMBERSHIP_SHA256:
            raise SystemExit("batch 01 original selection membership changed; define a new batch instead of refilling quotas")
        for path in sorted(copied):
            destination = vendor / path
            destination.parent.mkdir(parents=True, exist_ok=True)
            destination.write_bytes(sources.read(path))
        support_files = []
        for path in sorted(sources.required_resources()):
            destination = vendor / path
            destination.parent.mkdir(parents=True, exist_ok=True)
            destination.write_bytes(sources.read(path))
            support_files.append({
                "source": "vendor/" + path,
                "sha256": sha(sources.read(path)),
                "upstream_url": RAW_ROOT + path,
            })
        licenses = []
        for name in ["COPYRIGHT", "LICENSE-APACHE", "LICENSE-MIT"]:
            target = vendor / name
            target.write_bytes(sources.read(name))
            licenses.append({"source": "vendor/" + name, "sha256": sha(sources.read(name)), "upstream_url": RAW_ROOT + name})
        files = [sources.descriptor(path) for path in sorted(copied)]
        updates = classification_updates(sources, tests)
    manifest = {
        "schema_version": 1,
        "source_root": "vendor/tests",
        "upstream": {
            "repository": "https://github.com/rust-lang/rust",
            "commit": COMMIT,
            "version": "1.94.1",
            "archive_url": ARCHIVE_URL,
            "archive_sha256": ARCHIVE_SHA256,
            "licenses": licenses,
        },
        "selection_policy": {
            "candidate_pool": "all explicit run-pass/build-pass/run-fail primary .rs in the listed UI directories; no source rewriting",
            "directories_by_family": FAMILIES,
            "selected_quotas": QUOTAS,
            "priority": "assertion-bearing first, then increasing source byte length, then lexical upstream path",
            "classifier_version": CLASSIFIER_VERSION,
            "membership_policy": "batch 01 uses the original version-1 classifier and quotas; later capability corrections block members without replacement",
            "excluded_modes": ["check-pass", "check-fail", "build-fail", "compile-fail", "no explicit runtime/build mode"],
            "excluded_scope_reason": "first batch targets integer/control-flow/reference/aggregate/enum/call/drop/panic behavior; other suites/directories remain outside this declared candidate pool",
            "blocked_policy": "conservative lexical capability checks; raw protocol still must be interpreted per revision by the runner",
            "execution_policy": "selection is not a pass result; runner reports every selected revision and all blocked/excluded candidates separately",
            "required_support_files": [path.removeprefix("tests/") for path in REQUIRED_SUPPORT_FILES],
            "required_resource_directories": REQUIRED_RESOURCE_DIRECTORIES,
            "required_resource_files": REQUIRED_RESOURCE_FILES,
        },
        "counts": dict(collections.Counter(test["selection"]["status"] for test in tests)),
        "counts_by_family": {
            family: dict(collections.Counter(test["selection"]["status"] for test in tests if test["family"] == family))
            for family in FAMILIES
        },
        "selection_batch": {
            "id": SELECTION_BATCH_ID,
            "original_manifest_sha256": BASELINE_MANIFEST_SHA256,
            "original_classifier_version": 1,
            "original_selected_ids": original_members,
            "original_selected_ids_sha256": membership_sha256,
            "original_counts": dict(collections.Counter(test["initial_selection"]["status"] for test in tests)),
            "historical_results_policy": "prior results retain their original manifest fingerprint and counts; reclassification does not rewrite batch history",
        },
        "classifier_history": [
            {"version": 1, "manifest_sha256": BASELINE_MANIFEST_SHA256, "counts": {"selected": 160, "blocked": 73, "excluded": 194}},
            {"version": 2, "manifest_sha256": CLASSIFIER_V2_MANIFEST_SHA256, "counts": {"selected": 159, "blocked": 74, "excluded": 194}},
        ],
        "classification_updates": updates,
        "files": files,
        "support_files": support_files,
        "tests": tests,
    }
    manifest_path.write_text(json.dumps(manifest, indent=2, ensure_ascii=False) + "\n")
    return manifest


def verify_sources(archive_path: Path, output: Path) -> dict:
    if sha(archive_path.read_bytes()) != ARCHIVE_SHA256:
        raise SystemExit("archive SHA-256 differs from the pinned upstream archive")
    manifest = json.loads((output / "manifest.json").read_text())
    if manifest["upstream"]["commit"] != COMMIT or manifest["upstream"]["archive_sha256"] != ARCHIVE_SHA256:
        raise SystemExit("manifest upstream identity differs from the pinned source")
    support_files = {path.removeprefix("tests/") for path in REQUIRED_SUPPORT_FILES}
    if not support_files.issubset(item["source"] for item in manifest["files"]):
        raise SystemExit("manifest omits a required compiletest support file")
    original_members = [test["id"] for test in manifest["tests"] if test["initial_selection"]["status"] == "selected"]
    if sha(("\n".join(original_members) + "\n").encode()) != BASELINE_MEMBERSHIP_SHA256:
        raise SystemExit("batch 01 original selection membership changed")
    if manifest["selection_batch"]["original_selected_ids"] != original_members:
        raise SystemExit("batch 01 membership ledger differs from the test entries")
    if any(test["selection"]["status"] == "selected" and test["initial_selection"]["status"] != "selected" for test in manifest["tests"]):
        raise SystemExit("corrected classification unexpectedly selected a new batch member")
    with tarfile.open(archive_path, "r:gz") as archive:
        sources = Sources(archive)
        expected_support = {"vendor/" + path for path in sources.required_resources()}
        if {item["source"] for item in manifest.get("support_files", [])} != expected_support:
            raise SystemExit("manifest support resources differ from the required archive resources")
        for item in manifest["support_files"]:
            data = (output / item["source"]).read_bytes()
            original = sources.read(item["source"].removeprefix("vendor/"))
            if data != original or sha(data) != item["sha256"]:
                raise SystemExit(f"support resource differs from archive: {item['source']}")
        for item in manifest["files"]:
            data = (output / manifest["source_root"] / item["source"]).read_bytes()
            original = sources.read("tests/" + item["source"])
            if data != original or sha(data) != item["sha256"]:
                raise SystemExit(f"vendored file differs from archive: {item['source']}")
            if item["source"].endswith(".rs"):
                if directives(data.decode("utf-8")) != item.get("directives"):
                    raise SystemExit(f"directive ledger differs: {item['source']}")
        for item in manifest["upstream"]["licenses"]:
            data = (output / item["source"]).read_bytes()
            if data != sources.read(PurePosixPath(item["source"]).name) or sha(data) != item["sha256"]:
                raise SystemExit(f"license differs from archive: {item['source']}")
        for test in manifest["tests"]:
            data = (output / manifest["source_root"] / test["source"]).read_bytes()
            if sha(data) != test["source_sha256"] or directives(data.decode("utf-8")) != test["directives"]:
                raise SystemExit(f"test provenance differs: {test['source']}")
    return {
        "verified_files": len(manifest["files"]),
        "verified_support_files": len(manifest["support_files"]),
        "verified_tests": len(manifest["tests"]),
    }


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--archive", type=Path, required=True, help="cached archive path; downloaded from pinned URL if absent")
    parser.add_argument("--output", type=Path, default=DEFAULT_OUTPUT)
    parser.add_argument("--verify", action="store_true", help="verify existing imported files and directives against the original archive")
    options = parser.parse_args()
    if not options.archive.exists():
        options.archive.parent.mkdir(parents=True, exist_ok=True)
        with urllib.request.urlopen(ARCHIVE_URL, timeout=90) as response:
            options.archive.write_bytes(response.read())
    if options.verify:
        print(json.dumps(verify_sources(options.archive, options.output)))
        return
    manifest = import_sources(options.archive, options.output)
    print(json.dumps({"manifest": str(options.output / "manifest.json"), "counts": manifest["counts"], "files": len(manifest["files"])}))


if __name__ == "__main__":
    main()
