#!/usr/bin/env python3
"""Fetch the locked public crate, verify its registry checksum, and extract it."""
import hashlib
import json
import pathlib
import sys
import tarfile
import urllib.request

VERSION = "1.0.28"
SHA256 = "8a7852d02fc848982e0c167ef163aaff9cd91dc640ba85e263cb1ce46fae51cd"
work = pathlib.Path(sys.argv[1]).resolve()
archive = work / f"semver-{VERSION}.crate"
if not archive.exists():
    request = urllib.request.Request(
        f"https://static.crates.io/crates/semver/semver-{VERSION}.crate",
        headers={"User-Agent": "kfrc-poc-validation"},
    )
    with urllib.request.urlopen(request, timeout=120) as response:
        archive.write_bytes(response.read())
actual = hashlib.sha256(archive.read_bytes()).hexdigest()
if actual != SHA256:
    raise SystemExit(f"semver archive checksum mismatch: {actual}")
source = work / f"semver-{VERSION}"
if source.exists():
    raise SystemExit(f"refusing to reuse existing source directory: {source}")
with tarfile.open(archive) as package:
    for entry in package.getmembers():
        name = pathlib.PurePosixPath(entry.name)
        if name.is_absolute() or ".." in name.parts or name.parts[0] != source.name:
            raise SystemExit(f"invalid crate archive path: {entry.name}")
        if not (entry.isfile() or entry.isdir()):
            raise SystemExit(f"unsupported crate archive entry: {entry.name}")
    package.extractall(work)
files = {
    str(path.relative_to(source)): hashlib.sha256(path.read_bytes()).hexdigest()
    for path in sorted(source.rglob("*")) if path.is_file()
}
(work / "source.json").write_text(json.dumps({
    "name": "semver", "version": VERSION, "archive_sha256": actual,
    "registry_checksum": SHA256, "source_files_sha256": files,
}, indent=2) + "\n")
print(source)
