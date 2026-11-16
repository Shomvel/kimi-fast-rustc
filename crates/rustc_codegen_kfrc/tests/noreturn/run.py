"""Compare defined inputs and terminal process effects against LLVM."""
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import sys

backend, work = map(Path, sys.argv[1:3])
mode = sys.argv[3]
assert mode in ("require-pruning", "record"), mode
here = Path(__file__).resolve().parent
env = {key: value for key, value in os.environ.items() if not key.startswith("KFRC_")}
env["RUSTC_BOOTSTRAP"] = "1"
rustc = ["rustc", "+" + os.environ.get("KFRC_TOOLCHAIN", "1.94.1")]
flags = ["--edition=2021", "-Cpanic=abort", "-Coverflow-checks=off", "-Copt-level=3",
         "-Zmir-opt-level=0", "-Ccodegen-units=16", "-Clto=off", "-Cembed-bitcode=no"]
if os.environ.get("KFRC_SYSROOT"):
    flags.append("--sysroot=" + os.environ["KFRC_SYSROOT"])

def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()

def run(folder, label, command, expected=0, run_env=env):
    (folder / (label + ".command.json")).write_text(json.dumps(command, indent=2) + "\n")
    result = subprocess.run(command, env=run_env, capture_output=True)
    (folder / (label + ".stdout")).write_bytes(result.stdout)
    (folder / (label + ".stderr")).write_bytes(result.stderr)
    (folder / (label + ".result.json")).write_text(json.dumps({"exit_code": result.returncode}) + "\n")
    if result.returncode != expected:
        sys.stderr.buffer.write(result.stderr)
        raise SystemExit(f"{label}: exit {result.returncode}, expected {expected}")
    return result.stdout

summary = {"schema": 1, "mode": mode, "backend_sha256": sha(backend),
           "inputs": {path.name: sha(path) for path in here.iterdir() if path.is_file()}, "arms": {}}
outputs = None
for arm in ["llvm", "kfrc"]:
    folder = work / arm
    folder.mkdir()
    backend_flags = [] if arm == "llvm" else ["-Zcodegen-backend=" + str(backend)]
    run(folder, "library", [*rustc, *flags, *backend_flags, "--crate-type=rlib",
        "--crate-name=noreturn_representation", str(here / "library.rs"), "--out-dir=" + str(folder)],
        run_env=dict(env, KFRC_VERIFY="full", KFRC_CRANELIFT_VERIFY="1", KFRC_SPECIALIZATION_STATS="1"))
    library = folder / "libnoreturn_representation.rlib"
    run(folder, "driver", [*rustc, *flags, str(here / "driver.rs"),
        "--extern=noreturn_representation=" + str(library), "-o", str(folder / "driver")])
    actual = {name: run(folder, name, [str(folder / "driver"), name], expected=status).decode()
              for name, status in [("defined", 0), ("inline", 17), ("closed", 17)]}
    if outputs is None:
        outputs = actual
    assert actual == outputs, (arm, actual, outputs)
    listing = subprocess.check_output(["objdump", "-dr", "--demangle", "--no-show-raw-insn", str(library)], text=True)
    (folder / "disassembly.txt").write_text(listing)
    witnesses = {}
    for symbol in ["inline_exit", "closed_exit", "defined_guard"]:
        body = subprocess.check_output(["objdump", "-dr", "--no-show-raw-insn", "--disassemble=" + symbol, str(library)], text=True)
        (folder / (symbol + ".asm")).write_text(body)
        witnesses[symbol] = {"event_relocations": len(re.findall(r"R_X86_64_\S+\s+record_event(?:[+\-]|\s|$)", body)),
                             "unreachable_unchecked_relocations": len(re.findall(r"R_X86_64_.*unreachable_unchecked", body))}
    if arm == "kfrc" and mode == "require-pruning":
        assert witnesses["inline_exit"]["event_relocations"] == 2, witnesses
        assert witnesses["closed_exit"]["event_relocations"] == 1, witnesses
        assert not witnesses["defined_guard"]["unreachable_unchecked_relocations"], witnesses
    summary["arms"][arm] = {"output": actual, "witnesses": witnesses,
                            "rlib_sha256": sha(library), "driver_sha256": sha(folder / "driver")}
    (work / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")
print("noreturn: LLVM and KFRC agree on 4096 defined inputs, prior effects and exit status")
