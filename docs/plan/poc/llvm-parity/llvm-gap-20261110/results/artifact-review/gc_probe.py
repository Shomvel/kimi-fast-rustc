"""Probe ELF section GC with one retained function, not a runnable consumer.

Usage: python3 gc_probe.py EVIDENCE_RESULTS OUTPUT --lld LD_LLD --objdump LLVM_OBJDUMP
"""
import argparse
import hashlib
import json
from pathlib import Path
import re
import subprocess

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("evidence", type=Path)
parser.add_argument("output", type=Path)
parser.add_argument("--lld", default="ld.lld")
parser.add_argument("--objdump", default="llvm-objdump")
args = parser.parse_args()
args.output.mkdir(parents=True, exist_ok=True)
rows = json.loads((args.evidence / "static/functions.json").read_text())
root = next(r for r in rows if r["arm"] == "llvm" and "internal_decoded_len_estimate" in r["name"])
symbol = root["section"].removeprefix(".text.")
result = dict(function=root["name"], root=symbol,
              scope="relocatable single-object section-GC probe; not an executable or runtime measurement",
              linker_version=subprocess.check_output([args.lld, "--version"], text=True).strip(), arms={})
for arm in ("llvm", "data"):
    record = next(r for r in rows if r["arm"] == arm and r["name"] == root["name"])
    obj = next(p for p in (args.evidence / "suite" / arm / "base64/ref/objects").glob("*.o")
               if hashlib.sha256(p.read_bytes()).hexdigest() == record["object_sha256"])
    destination = args.output / (arm + ".o")
    command = [args.lld, "-r", "--gc-sections", "-u", symbol, str(obj), "-o", str(destination)]
    subprocess.run(command, check=True)
    headers = subprocess.check_output([args.objdump, "-h", str(destination)], text=True)
    (args.output / (arm + ".sections.txt")).write_text(headers)
    sections = []
    for line in headers.splitlines():
        if match := re.match(r"\s*\d+\s+(\.\S+)\s+([0-9a-f]+)\s+[0-9a-f]+(?:\s+(\w+))?", line):
            sections.append(dict(name=match[1], bytes=int(match[2], 16), kind=match[3]))
    result["arms"][arm] = dict(command=command, input_sha256=record["object_sha256"],
                                output_sha256=hashlib.sha256(destination.read_bytes()).hexdigest(),
                                object_bytes=destination.stat().st_size, root_function_bytes=record["bytes"],
                                text_bytes=sum(s["bytes"] for s in sections if s["kind"] == "TEXT"),
                                sections=sections)
(args.output / "summary.json").write_text(json.dumps(result, indent=2) + "\n")
print({arm: data["text_bytes"] for arm, data in result["arms"].items()})
