#!/usr/bin/env python3
"""Archive only compiler sources and test inputs, with per-file provenance."""
import argparse
import hashlib
import io
import json
from pathlib import Path
import subprocess
import tarfile

parser = argparse.ArgumentParser()
parser.add_argument("output", type=Path)
args = parser.parse_args()
root = Path(__file__).resolve().parents[2]
args.output.mkdir(parents=True, exist_ok=True)
files = [root / "Cargo.toml", root / "Cargo.lock"]
for directory in ("src", "crates", "tests", "examples", "docs", "scripts/poc-validation", "scripts/rustc-upstream"):
    base = root / directory
    if base.exists():
        files.extend(p for p in base.rglob("*") if p.is_file()
                     and not any(part in ("target", "__pycache__", ".git") for part in p.relative_to(root).parts)
                     and p.suffix not in (".pyc", ".so", ".dylib"))
files = sorted(set(p for p in files if p.exists()))
# Hash and archive the same bytes even while other agents are editing files.
contents = {path: path.read_bytes() for path in files}
manifest = {str(path.relative_to(root)): hashlib.sha256(content).hexdigest()
            for path, content in contents.items()}
archive = args.output / "source.tar.gz"
with tarfile.open(archive, "w:gz") as tar:
    for path in files:
        info = tar.gettarinfo(str(path), arcname=str(path.relative_to(root)))
        info.type = tarfile.REGTYPE
        info.linkname = ""
        info.size = len(contents[path])
        tar.addfile(info, io.BytesIO(contents[path]))
provenance = {
    "commit": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=root, text=True).strip(),
    "branch": subprocess.check_output(["git", "branch", "--show-current"], cwd=root, text=True).strip(),
    "archive_sha256": hashlib.sha256(archive.read_bytes()).hexdigest(),
    "files": manifest,
}
(args.output / "source.json").write_text(json.dumps(provenance, indent=2) + "\n")
(args.output / "source.patch").write_bytes(subprocess.check_output(["git", "diff", "HEAD", "--", "src", "crates", "Cargo.toml", "Cargo.lock"], cwd=root))
print(json.dumps({"archive": str(archive), "sha256": provenance["archive_sha256"], "files": len(files)}))
