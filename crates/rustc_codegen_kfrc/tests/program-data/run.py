"""Run a backend differential and inspect actual ELF definitions in each rlib."""
import collections
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys

backend, work = map(Path, sys.argv[1:3])
mode = sys.argv[3]
assert mode in ("require-unique", "record"), mode
here = Path(__file__).resolve().parent
env = {key: value for key, value in os.environ.items() if not key.startswith("KFRC_")}
env["RUSTC_BOOTSTRAP"] = "1"
rustc = ["rustc", "+" + os.environ.get("KFRC_TOOLCHAIN", "1.94.1")]
flags = ["--edition=2021", "-Cpanic=abort", "-Coverflow-checks=off",
         "-Copt-level=3", "-Zmir-opt-level=0", "-Clto=off", "-Cembed-bitcode=no"]
if os.environ.get("KFRC_SYSROOT"):
    flags.append("--sysroot=" + os.environ["KFRC_SYSROOT"])

def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()

def run(folder, label, command, run_env=env):
    (folder / (label + ".command.json")).write_text(json.dumps(command, indent=2) + "\n")
    result = subprocess.run(command, env=run_env, capture_output=True)
    (folder / (label + ".stdout")).write_bytes(result.stdout)
    (folder / (label + ".stderr")).write_bytes(result.stderr)
    (folder / (label + ".result.json")).write_text(json.dumps({"exit_code": result.returncode}) + "\n")
    if result.returncode:
        sys.stderr.buffer.write(result.stderr)
        raise SystemExit(result.returncode)
    return result.stdout

def inspect(folder, name, require_unique):
    archive = folder / ("lib" + name + ".rlib")
    objects = folder / (name + "-objects")
    objects.mkdir()
    members = subprocess.check_output(["ar", "t", str(archive)], text=True).splitlines()
    definitions = collections.defaultdict(list)
    hidden_functions = []
    anonymous = []
    for member in members:
        if not member.endswith(".o"):
            continue
        assert Path(member).name == member, member
        path = objects / member
        path.write_bytes(subprocess.check_output(["ar", "p", str(archive), member]))
        symbols = subprocess.check_output(["readelf", "--wide", "--symbols", str(path)], text=True)
        path.with_suffix(".symbols").write_text(symbols)
        for line in symbols.splitlines():
            parts = line.split()
            if len(parts) < 8 or not parts[0].rstrip(":").isdigit() or parts[6] in ("UND", "ABS"):
                continue
            _, _, size, kind, binding, visibility, _, symbol = parts[:8]
            if kind == "FUNC":
                definitions[symbol].append({"member": member, "size": int(size),
                                            "binding": binding, "visibility": visibility})
                if binding == "GLOBAL" and visibility == "HIDDEN":
                    hidden_functions.append(symbol)
            if symbol.startswith(".Lkfrc_const_"):
                anonymous.append({"member": member, "symbol": symbol, "size": int(size)})
    duplicates = {symbol: entries for symbol, entries in definitions.items() if len(entries) > 1}
    if require_unique:
        assert not duplicates, (name, duplicates)
    # A canonical constant appears at most once in each using object.
    keys = [(entry["member"], entry["symbol"]) for entry in anonymous]
    assert len(keys) == len(set(keys)), (name, "duplicate canonical constant in one CGU")
    return {"rlib_sha256": sha(archive), "functions": len(definitions),
            "extra_function_definitions": sum(len(entries) - 1 for entries in duplicates.values()),
            "duplicates": duplicates, "hidden_functions": hidden_functions,
            "anonymous_constants": anonymous}

summary = {"schema": 1, "backend_sha256": sha(backend), "mode": mode,
           "inputs": {path.name: sha(path) for path in here.iterdir() if path.is_file()}, "arms": {}}
summary["rustc"] = subprocess.check_output([*rustc, "-Vv"], env=env, text=True)
expected = None
for arm, cgus in [("llvm", 16), ("kfrc-cgu1", 1), ("kfrc-cgu16", 16)]:
    folder = work / arm
    folder.mkdir()
    backend_flags = [] if arm == "llvm" else ["-Zcodegen-backend=" + str(backend)]
    library_env = dict(env, KFRC_VERIFY="full", KFRC_CRANELIFT_VERIFY="1")
    if arm != "llvm":
        library_env["KFRC_CGSCC_STATS"] = "1"
    for name, source in [("program_shared", "shared.rs"), ("program_left", "branch.rs"),
                         ("program_right", "branch.rs")]:
        command = [*rustc, *flags, *backend_flags, "-Ccodegen-units=" + str(cgus),
                   "--crate-type=rlib", "--crate-name=" + name, str(here / source),
                   "--out-dir=" + str(folder), "-Ldependency=" + str(folder)]
        if source == "branch.rs":
            command.append("--extern=program_shared=" + str(folder / "libprogram_shared.rlib"))
        run(folder, name, command, library_env)
    command = [*rustc, *flags, "-Ccodegen-units=1", str(here / "driver.rs"),
               "-Ldependency=" + str(folder), "-o", str(folder / "driver")]
    for name in ("program_shared", "program_left", "program_right"):
        command.append("--extern=" + name + "=" + str(folder / ("lib" + name + ".rlib")))
    run(folder, "driver", command)
    output = run(folder, "execute", [str(folder / "driver")])
    if expected is None:
        expected = output
    assert output == expected, (arm, output, expected)
    libraries = {name: inspect(folder, name, arm != "llvm" and mode == "require-unique")
                 for name in ("program_shared", "program_left", "program_right")}
    summary["arms"][arm] = {"output": output.decode(), "driver_sha256": sha(folder / "driver"),
                            "libraries": libraries}
    (work / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")
print(expected.decode(), end="")
print("program-data: LLVM, single-CGU and multi-CGU libraries agree")
