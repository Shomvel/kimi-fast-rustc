"""Checks outside compiletest: dual-backend behavior and object provenance."""

import hashlib
import json
from pathlib import Path
import struct
import subprocess


def sha256(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def captures(logs):
    result = {}
    for path in sorted((Path(logs) / "runs").glob("*/run.json")):
        record = json.loads(path.read_text())
        key = record["executable_key"]
        result.setdefault(key, []).append((record, path.parent))
    return result


def normalized_output(data, record, build_root):
    # Only adapter-controlled paths differ between arms. Do not discard PIDs,
    # addresses, panic messages, numbers, or arbitrary user output.
    replacements = [
        (record["executable"].encode(), b"$TEST_BINARY"),
        (str(Path(build_root).resolve()).encode(), b"$BUILD_ROOT"),
    ]
    for source, target in replacements:
        data = data.replace(source, target)
    return data


def compare_runs(llvm_logs, kfrc_logs, llvm_build, kfrc_build, *, expected_runtime):
    llvm = captures(llvm_logs)
    kfrc = captures(kfrc_logs)
    differences = []
    if llvm.keys() != kfrc.keys():
        differences.append("runtime executable identities differ")
    if expected_runtime and not llvm:
        differences.append("no LLVM runtime capture; test was not executed")
    compared = 0
    for key in sorted(llvm.keys() & kfrc.keys()):
        if len(llvm[key]) != len(kfrc[key]):
            differences.append(f"{key}: runtime invocation count differs")
            continue
        for index, ((left, left_dir), (right, right_dir)) in enumerate(zip(llvm[key], kfrc[key])):
            prefix = f"{key} invocation {index}"
            if left["returncode"] != right["returncode"]:
                differences.append(f"{prefix}: exit/signal differs")
            for stream in ("stdout", "stderr"):
                left_bytes = normalized_output((left_dir / stream).read_bytes(), left, llvm_build)
                right_bytes = normalized_output((right_dir / stream).read_bytes(), right, kfrc_build)
                if left_bytes != right_bytes:
                    differences.append(f"{prefix}: {stream} differs")
            compared += 1
    return {"invocations_compared": compared, "differences": differences}


def object_provenance(build_root, logs):
    objects = sorted(Path(build_root).rglob("*.o"))
    records = []
    for index, path in enumerate(objects):
        process = subprocess.run(["readelf", "-Ws", str(path)], capture_output=True)
        symbol_file = Path(logs) / f"object-{index:04}.symbols.txt"
        symbol_file.write_bytes(process.stdout + process.stderr)
        is_kfrc = process.returncode == 0 and any(
            b"FILE" in line and line.split()[-1:] == [b"kfrc"]
            for line in process.stdout.splitlines()
        )
        # rustc's linker writes a small symbols.o to retain/export symbols.
        # It is not a codegen unit. Exempt only this known artifact when its
        # ELF sections prove it contains no executable bytes; every other
        # object, including all auxiliaries and rcgu objects, needs our writer.
        linker_metadata = (path.name == "symbols.o" and path.parent.name.startswith("rustc")
                           and executable_sections_empty(path))
        if linker_metadata:
            sections = subprocess.run(["readelf", "-SW", str(path)], capture_output=True)
            symbol_file.with_suffix(".sections.txt").write_bytes(sections.stdout + sections.stderr)
        records.append({
            "path": str(path.relative_to(build_root)),
            "sha256": sha256(path),
            "kfrc_writer": is_kfrc,
            "kind": "rustc-linker-metadata" if linker_metadata else "codegen",
            "symbols": str(symbol_file),
        })
    return {
        "ok": any(record["kfrc_writer"] for record in records)
              and all(record["kfrc_writer"] or record["kind"] == "rustc-linker-metadata" for record in records),
        "objects": records,
    }


def executable_sections_empty(path):
    """Prove absence of executable payload in a native ELF64 little-endian file."""
    data = path.read_bytes()
    if data[:6] != b"\x7fELF\x02\x01":
        return False
    try:
        offset = struct.unpack_from("<Q", data, 40)[0]
        entry_size, count = struct.unpack_from("<HH", data, 58)
        if entry_size < 64 or count == 0 or offset + entry_size * count > len(data):
            return False
        for index in range(count):
            header = offset + index * entry_size
            flags = struct.unpack_from("<Q", data, header + 8)[0]
            size = struct.unpack_from("<Q", data, header + 32)[0]
            if flags & 4 and size:
                return False
    except struct.error:
        return False
    return True
