#!/usr/bin/env python3
"""Check behavior, compiler origin, semantic stages, and whole-crate rejection."""
import json
from pathlib import Path
import re
import sys

root = Path(sys.argv[1])
summary = {
    "scope": "Rust frontend -> sole kfrc semantic library pipeline -> x86; LLVM ABI provider and driver",
    "arms": {},
    "negative_probes": {},
    "performance_measured": False,
}
reference = (root / "llvm-execute.stdout").read_bytes()
assert reference, "driver produced no observable output"
for arm in ("llvm", "semantic-off", "semantic-on"):
    for step in ("library", "driver-build", "execute"):
        assert json.loads((root / f"{arm}-{step}.result.json").read_text())["exit_code"] == 0
    assert (root / f"{arm}-execute.stdout").read_bytes() == reference
    symbols = (root / arm / "object-symbols.txt").read_text()
    writer = bool(re.search(r"FILE\s+LOCAL\s+DEFAULT\s+ABS\s+kfrc\b", symbols))
    assert writer == (arm != "llvm"), f"{arm}: wrong object writer"
    row = {"output_lines": len(reference.splitlines()), "kfrc_object_writer": writer}
    if arm.startswith("semantic-"):
        graph_log = (root / f"{arm}-library.stderr").read_text()
        inputs = re.findall(r"kfrc semantic input ([^\n]+)", graph_log)
        layouts = re.findall(r"kfrc layout output ([^\n]+)", graph_log)
        assert inputs and set(layouts) <= set(inputs), "emitted layout graph lacks a semantic input"
        omitted = set(inputs) - set(layouts)
        assert not omitted or arm == "semantic-on", "unoptimized graph disappeared"
        off_symbols = (root / "semantic-off" / "object-symbols.txt").read_text()
        for marker in omitted:
            symbol = marker.removesuffix(":")
            assert re.search(r"\bFUNC\s+LOCAL\s+\w+\s+\S+\s+" + re.escape(symbol) + r"$", off_symbols, re.M), "omitted symbol was not internal in the unoptimized object"
            assert not re.search(r"\s" + re.escape(symbol) + r"$", symbols, re.M), "omitted function still has an object definition or unresolved reference"
        row["internal_symbols_omitted_after_optimization"] = sorted(marker.removesuffix(":") for marker in omitted)
        assert len(inputs) >= 13, "missing expected library functions"
        if arm == "semantic-on":
            inlined = [int(n) for n in re.findall(r"calls_inlined: (\d+)", graph_log)]
            assert inlined and sum(inlined) > 0, "eligible same-crate helper was not inlined"
            row["calls_inlined"] = sum(inlined)
        row["semantic_functions"] = len(inputs)
        row["input_symbols"] = sorted(inputs)
        semantic_ops = {"Aggregate", "Field", "InsertField", "MakeVariant", "VariantOf", "VariantField", "WidePtr", "Discriminant"}
        memory_ops = {"StackSlot", "Load", "Store", "PtrOffset"}
        graphs = {}
        for match in re.finditer(r"^graph ([^\n]+) \((Specialized|Layout), (\w+)\) \{\n(.*?)^\}", graph_log, re.M | re.S):
            symbol, level, state, body = match.groups()
            assert state == "Verified", f"unverified {level} dump for {symbol}"
            ops = re.findall(r"^  n\d+ = ([A-Za-z]+)", body, re.M)
            counts = {op: ops.count(op) for op in sorted(set(ops))}
            if level == "Specialized":
                assert not memory_ops.intersection(ops), f"early physical memory operations in {symbol}"
            else:
                assert not semantic_ops.intersection(ops), f"residual semantic operation after lowering: {symbol}"
            graphs.setdefault(symbol, []).append({"level": level, "nodes": len(ops), "opcodes": counts})
        for symbol, dumps in graphs.items():
            expected_stages = ["Specialized"] if symbol + ":" in omitted else ["Specialized", "Specialized", "Layout"]
            assert [d["level"] for d in dumps] == expected_stages, f"missing stage dump for {symbol}"
            if "tuple_field" in symbol:
                if arm == "semantic-on":
                    assert not {"Aggregate", "Field"}.intersection(dumps[1]["opcodes"]), "tuple field was not folded before layout"
                assert not {"Aggregate", "Field", "StackSlot", "Load", "Store"}.intersection(dumps[2]["opcodes"]), "tuple field acquired unnecessary layout storage"
        assert len(graphs) == len(inputs), "dump/function marker count mismatch"
        row["graphs"] = graphs
        row["stage_totals"] = {
            stage: {"nodes": sum(dumps[i]["nodes"] for dumps in graphs.values() if len(dumps) > i),
                    "opcodes": {op: sum(dumps[i]["opcodes"].get(op, 0) for dumps in graphs.values() if len(dumps) > i)
                                for op in sorted(semantic_ops | memory_ops | {"Phi"})}}
            for i, stage in enumerate(("semantic_input", "semantic_after_passes", "layout_output"))
        }
    summary["arms"][arm] = row
for probe in ("union", "tls", "simd"):
    status = json.loads((root / f"reject-{probe}.result.json").read_text())["exit_code"]
    diagnostic = (root / f"reject-{probe}.stderr").read_text()
    assert status != 0 and "kfrc:" in diagnostic and "cannot compile" in diagnostic
    assert not list((root / f"reject-{probe}").glob("*.rlib"))
    summary["negative_probes"][probe] = {"exit_code": status, "rlib_emitted": False}
(root / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")
print(f"PASS: three arms, {len(reference.splitlines())} lines each, cross-backend Pair/Indirect calls, explicit S->L, borrow/index/enum/niche behavior, three scope rejections")
