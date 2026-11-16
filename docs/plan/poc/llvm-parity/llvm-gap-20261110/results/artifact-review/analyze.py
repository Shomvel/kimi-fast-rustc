"""Reinspect verified ELF objects without rebuilding or rerunning benchmarks.

Usage: python3 analyze.py EVIDENCE_RESULTS OUTPUT_DIR --objdump LLVM_OBJDUMP
The input is results/ from the division-inline evidence archive.
"""
import argparse
from collections import Counter
import hashlib
import json
from pathlib import Path
import re
import subprocess


def write(path, value):
    path.write_text(json.dumps(value, indent=2) + "\n")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("evidence", type=Path)
    parser.add_argument("output", type=Path)
    parser.add_argument("--objdump", default="llvm-objdump")
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    records = json.loads((args.evidence / "static/functions.json").read_text())
    records = [r for r in records if r["arm"] in ("llvm", "data")]
    instructions = re.compile(r"^\s*([0-9a-f]+):\s+((?:[0-9a-f]{2}\s+)+)\s*([a-z][a-z0-9]*)\s*(.*)$")
    objects = {}
    manifest = []
    for arm in ("llvm", "data"):
        for crate in ("semver", "base64", "unicode-segmentation", "humantime"):
            for path in sorted((args.evidence / "suite" / arm / crate / "ref/objects").glob("*.o")):
                digest = hashlib.sha256(path.read_bytes()).hexdigest()
                text = subprocess.check_output(
                    [args.objdump, "-dr", "--demangle", "--x86-asm-syntax=intel", str(path)], text=True
                )
                destination = args.output / "asm" / arm / crate / (path.stem + ".asm")
                destination.parent.mkdir(parents=True, exist_ok=True)
                destination.write_text(text)
                sections = {}
                section = None
                for line in text.splitlines():
                    if line.startswith("Disassembly of section "):
                        section = line.removeprefix("Disassembly of section ").removesuffix(":")
                        sections.setdefault(section, [])
                    elif (match := instructions.match(line)) and section is not None:
                        address, code, op, operands = match.groups()
                        sections[section].append((int(address, 16), len(code.split()), op, operands))
                objects[arm, crate, digest] = sections
                manifest.append(dict(arm=arm, crate=crate, object=path.name, sha256=digest))
    for record in records:
        section = objects[record["arm"], record["crate"], record["object_sha256"]][record["section"]]
        start = record["start"]
        body = [i for i in section if start <= i[0] < start + record["bytes"]]
        assert len(body) == record["counts"]["instructions"], (record["name"], len(body), record["counts"])
        counters = Counter()
        for address, size, op, operands in body:
            if op.startswith("j"):
                counters["unconditional_jumps" if op == "jmp" else "conditional_jumps"] += 1
                target = re.match(r"0x([0-9a-f]+)", operands)
                if target and int(target[1], 16) == address + size:
                    counters["jump_to_next_instruction"] += 1
            if op.startswith("set"):
                counters["setcc"] += 1
            if op.startswith("cmov"):
                counters["cmov"] += 1
            if op == "lea" and re.search(r"\[(?:r|e)(?:bp|sp)(?:\s|\])", operands):
                counters["stack_address_lea"] += 1
            if op.startswith("mov") and re.search(r"-922337203685477580[0-8]\b", operands):
                counters["niche_tag_constant_materializations"] += 1
            if re.search(r"\b[xyz]mm\d+\b", operands):
                counters["vector_register_instructions"] += 1
        reservations = [int(m[1], 0) for _, _, op, operands in body if op == "sub"
                        and (m := re.fullmatch(r"rsp, (0x[0-9a-f]+|[0-9]+)", operands))]
        record["inspection"] = dict(counters)
        record["inspection"]["stack_sub_rsp_bytes"] = reservations
    write(args.output / "functions.json", records)
    write(args.output / "objects.json", manifest)
    summaries = []
    for arm in ("llvm", "data"):
        for crate in ("semver", "base64", "unicode-segmentation", "humantime"):
            rows = [r for r in records if r["arm"] == arm and r["crate"] == crate]
            totals = Counter()
            for row in rows:
                totals.update(row["counts"])
            summaries.append(dict(arm=arm, crate=crate, function_ranges=len(rows),
                                  summed_function_bytes=sum(r["bytes"] for r in rows), counts=dict(totals)))
    write(args.output / "library-functions.json", summaries)
    timed = json.loads((args.evidence / "confirmed-summary.json").read_text())
    dynamic = []
    for row in timed["workloads"]:
        raw = json.loads((args.evidence / "suite/runtime" / (row["crate"] + ".json")).read_text())
        arms = raw["workloads"][row["workload"]]["arms"]
        a, b = arms["data"], arms["llvm"]
        dynamic.append(dict(crate=row["crate"], workload=row["workload"],
                            paired_time_ratio=row["candidate_over_llvm"],
                            instruction_ratio=a["instructions"] / b["instructions"],
                            cycle_ratio=a["cycles"] / b["cycles"],
                            data_cpi=a["cycles"] / a["instructions"],
                            llvm_cpi=b["cycles"] / b["instructions"]))
    write(args.output / "dynamic.json", dynamic)
    print(f"Verified {len(manifest)} objects and {len(records)} function instruction counts")


if __name__ == "__main__":
    main()
