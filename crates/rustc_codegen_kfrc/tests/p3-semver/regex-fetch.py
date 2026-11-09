#!/usr/bin/env python3
"""Fixed regex-syntax source for the later public-API differential."""
import hashlib
import json
import pathlib
import sys
import tarfile
import urllib.request

name = "regex-syntax-0.8.8"
expected = "7a2d987857b319362043e95f5353c0535c1f58eec5336fdfcf626430af7def58"
work = pathlib.Path(sys.argv[1]).resolve()
archive = work / (name + ".crate")
if not archive.exists():
    with urllib.request.urlopen("https://static.crates.io/crates/regex-syntax/" + archive.name, timeout=120) as response:
        archive.write_bytes(response.read())
actual = hashlib.sha256(archive.read_bytes()).hexdigest()
if actual != expected:
    raise SystemExit(f"regex-syntax archive checksum mismatch: {actual}")
source = work / name
if source.exists():
    raise SystemExit(f"refusing to reuse source directory: {source}")
with tarfile.open(archive) as package:
    for member in package.getmembers():
        path = pathlib.PurePosixPath(member.name)
        if path.is_absolute() or ".." in path.parts or path.parts[0] != name or not (member.isfile() or member.isdir()):
            raise SystemExit(f"invalid crate archive entry: {member.name}")
    package.extractall(work)
(work / "source.json").write_text(json.dumps({
    "name": "regex-syntax", "version": "0.8.8", "archive_sha256": actual,
    "source_files_sha256": {str(path.relative_to(source)): hashlib.sha256(path.read_bytes()).hexdigest()
                            for path in sorted(source.rglob("*")) if path.is_file()},
}, indent=2) + "\n")
print(source)
