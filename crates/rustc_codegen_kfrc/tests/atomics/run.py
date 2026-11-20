"""Compile with each backend, link into LLVM harness and compare behavior."""
import json
import os
from pathlib import Path
import subprocess
import sys

backend = str(Path(sys.argv[1]).resolve())
work = Path(sys.argv[2]).resolve()
here = Path(__file__).resolve().parent
env = dict(os.environ, RUSTC_BOOTSTRAP="1", KFRC_MACHINE="cranelift", KFRC_CRANELIFT_VERIFY="1")
env.pop("KFRC_GAP_SURVEY", None)
outputs = []
for arm in ("llvm", "kfrc"):
    output = work / arm
    output.mkdir(parents=True, exist_ok=False)
    flags = ["--edition=2024", "-Cpanic=abort", "-Zmir-opt-level=0", "-Copt-level=2", "-Ccodegen-units=4"]
    extra = ["-Zcodegen-backend=" + backend] if arm == "kfrc" else []
    commands = [
        ["rustc", *flags, *extra, "--crate-type=rlib", "--crate-name=atomics", str(here / "library.rs"), "--out-dir=" + str(output)],
        ["rustc", *flags, str(here / "driver.rs"), "--extern=atomics=" + str(output / "libatomics.rlib"), "-o", str(output / "driver")],
        [str(output / "driver")],
    ]
    for index, command in enumerate(commands):
        (output / f"{index}.command.json").write_text(json.dumps(command))
        result = subprocess.run(command, env=env, capture_output=True, timeout=180)
        (output / f"{index}.stdout").write_bytes(result.stdout)
        (output / f"{index}.stderr").write_bytes(result.stderr)
        (output / f"{index}.exit").write_text(str(result.returncode))
        assert result.returncode == 0, result.stderr.decode(errors="replace")
    outputs.append(result.stdout)
assert outputs[0] == outputs[1]
print(outputs[1].decode(), end="")
