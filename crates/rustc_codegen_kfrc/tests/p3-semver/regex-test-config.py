#!/usr/bin/env python3
"""Resolve the pinned crate's actual default feature closure and source hashes."""
import hashlib
import json
import pathlib
import sys
import tomllib

work = pathlib.Path(sys.argv[1]).resolve()
source = work / "regex-syntax-0.8.8"
record = json.loads((work / "source.json").read_text())
actual = {str(path.relative_to(source)): hashlib.sha256(path.read_bytes()).hexdigest()
          for path in sorted(source.rglob("*")) if path.is_file()}
if actual != record["source_files_sha256"]:
    raise SystemExit("upstream regex-syntax sources changed after extraction")
manifest = tomllib.loads((source / "Cargo.toml").read_text())
if manifest["package"]["version"] != "0.8.8":
    raise SystemExit("unexpected regex-syntax version")
features = manifest["features"]
enabled = {"default"}
queue = ["default"]
while queue:
    for feature in features[queue.pop()]:
        if feature.startswith("dep:") or "/" in feature:
            raise SystemExit(f"default feature enables an external dependency: {feature}")
        if feature not in enabled:
            enabled.add(feature)
            queue.append(feature)
resolved = sorted(enabled)
(work / "test-features.json").write_text(json.dumps({
    "declared_default": features["default"], "enabled": resolved,
    "cfg_test": True, "target": "x86_64-unknown-linux-gnu",
    "source_files_verified": len(actual),
}, indent=2) + "\n")
print("\n".join(resolved))
