#!/usr/bin/env python3
"""Check P1's own local-exec TLS symbols, rather than std's TLS presence."""
import argparse
import hashlib
import json
from pathlib import Path
import re
import subprocess


def readelf(path, mode):
    return subprocess.check_output(["readelf", mode, str(path)], text=True)


def symbols(text):
    rows = {}
    for line in text.splitlines():
        fields = line.split()
        if len(fields) >= 8 and fields[0].rstrip(":").isdigit():
            rows[fields[7]] = {
                "value": fields[1], "size": fields[2], "type": fields[3],
                "binding": fields[4], "visibility": fields[5], "section": fields[6],
            }
    return rows


def sections(text):
    rows = {}
    pattern = re.compile(
        r"\[\s*(\d+)\]\s+(\S+)\s+(\S+)\s+\S+\s+\S+\s+\S+\s+\S+\s+(\S+)"
    )
    for line in text.splitlines():
        match = pattern.search(line)
        if match:
            index, name, kind, flags = match.groups()
            rows[index] = {"name": name, "kind": kind, "flags": flags}
    return rows


parser = argparse.ArgumentParser()
parser.add_argument("memory", type=Path, help="completed P1 memory result directory")
args = parser.parse_args()
root = args.memory.resolve()
out = root / "tls-elf"
out.mkdir(exist_ok=True)
required = ("INITIALIZED", "NONZERO", "ZERO", "INTERIOR")
report = {"model": "x86_64 Linux local-exec, R_X86_64_TPOFF32", "objects": [], "own_symbols": {}}
errors = []
for path in sorted((root / "kfrc/objects").glob("*.o")):
    symtext = readelf(path, "-Ws")
    symrows = symbols(symtext)
    if not any(row["type"] == "FILE" and name == "kfrc" for name, row in symrows.items()):
        continue
    sectext = readelf(path, "-WS")
    reltext = readelf(path, "-Wr")
    distext = subprocess.check_output(["objdump", "-drw", str(path)], text=True)
    for suffix, content in (("symbols", symtext), ("sections", sectext), ("relocations", reltext), ("disassembly", distext)):
        (out / f"{path.name}.{suffix}.txt").write_text(content)
    secrows = sections(sectext)
    relocations = [line.strip() for line in reltext.splitlines() if "R_X86_64_TPOFF32" in line]
    tlsrows = {name: row for name, row in symrows.items() if row["type"] == "TLS"}
    report["objects"].append({"path": str(path), "sha256": hashlib.sha256(path.read_bytes()).hexdigest(), "tls_symbols": tlsrows, "local_exec_relocations": relocations})
    for identifier in required:
        fragment = f"{len(identifier)}{identifier}"
        for name, row in tlsrows.items():
            if "thread_local" not in name or fragment not in name or not row["section"].isdigit():
                continue
            section = secrows.get(row["section"], {})
            references = [line for line in relocations if name in line]
            report["own_symbols"][identifier] = {"symbol": name, "object": str(path), "section": section, "relocations": references}
            if "T" not in section.get("flags", ""):
                errors.append(f"{identifier}: defining section lacks SHF_TLS")
            if not references:
                errors.append(f"{identifier}: no own-symbol R_X86_64_TPOFF32 reference")

driver = root / "kfrc/driver"
headers = readelf(driver, "-Wl")
finaltext = readelf(driver, "-Ws")
(out / "driver.program-headers.txt").write_text(headers)
(out / "driver.symbols.txt").write_text(finaltext)
finalrows = symbols(finaltext)
report["driver_sha256"] = hashlib.sha256(driver.read_bytes()).hexdigest()
report["pt_tls"] = [line.strip() for line in headers.splitlines() if re.match(r"\s*TLS\s", line)]
if not report["pt_tls"]:
    errors.append("final executable lacks PT_TLS")
for identifier in required:
    own = report["own_symbols"].get(identifier)
    if own is None:
        errors.append(f"{identifier}: missing defined TLS symbol in a FILE kfrc object")
        continue
    final = finalrows.get(own["symbol"], {})
    own["final_symbol"] = final
    if final.get("type") != "TLS" or not final.get("section", "").isdigit():
        errors.append(f"{identifier}: same symbol is not defined TLS in final executable")
report["errors"] = errors
report["status"] = "FAIL" if errors else "PASS"
(out / "summary.json").write_text(json.dumps(report, indent=2) + "\n")
print(json.dumps({"status": report["status"], "own_symbols": len(report["own_symbols"]), "kfrc_objects": len(report["objects"]), "errors": errors}))
raise SystemExit(bool(errors))
