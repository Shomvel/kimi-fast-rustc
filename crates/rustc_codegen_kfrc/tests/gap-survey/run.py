"""Native Linux integration gate: continuation, completeness and no output."""
import json
import os
from pathlib import Path
import subprocess
import sys

backend = str(Path(sys.argv[1]).resolve())
work = Path(sys.argv[2]).resolve()
here = Path(__file__).resolve().parent
env = dict(os.environ, RUSTC_BOOTSTRAP="1", KFRC_MACHINE="cranelift")
env.pop("KFRC_GAP_SURVEY", None)
for fixture in ("supported", "mixed", "nonunit"):
    for mode in ("default", "survey"):
        output = work / fixture / mode
        output.mkdir(parents=True, exist_ok=False)
        run_env = dict(env)
        if mode == "survey":
            run_env["KFRC_GAP_SURVEY"] = str(output / "reports")
        crate_type = "bin" if fixture == "nonunit" else "lib"
        command = ["rustc", "--edition=2024", "--crate-type=" + crate_type, "--emit=obj",
                   "-Cpanic=abort", "-Zmir-opt-level=0", "-Zcodegen-backend=" + backend,
                   "--out-dir=" + str(output), str(here / (fixture + ".rs"))]
        result = subprocess.run(command, env=run_env, capture_output=True)
        (output / "stdout").write_bytes(result.stdout)
        (output / "stderr").write_bytes(result.stderr)
        (output / "command.json").write_text(json.dumps(command))
        (output / "exit").write_text(str(result.returncode))
        if mode == "default":
            assert (result.returncode == 0) == (fixture == "supported")
            assert bool(list(output.glob("*.o"))) == (fixture == "supported")
            assert not (output / "reports").exists()
        else:
            assert result.returncode != 0
            assert not list(output.glob("*.o"))
            files = list((output / "reports").glob("*.jsonl"))
            assert len(files) == 1
            records = [json.loads(line) for line in files[0].read_text().splitlines()]
            assert records[0]["kind"] == "begin"
            assert records[-1]["kind"] == "summary" and records[-1]["status"] == "complete"
            functions = {r["symbol"]: r for r in records if r["kind"] == "function"}
            if fixture == "nonunit":
                assert any(r["kind"] == "entry" and r["status"] == "error" for r in records)
                assert functions, "entry failure must not prevent mono function survey"
                continue
            assert functions["supported"]["status"] == "ok"
            if fixture == "mixed":
                assert functions["floating"]["status"] == "error"
                errors = [r for r in functions.values() if r["status"] == "error"]
                assert functions["assembly"]["status"] == "error"
                assert len(errors) >= 2
                observed = {r["message"] for r in records if r["kind"] == "intrinsic"}
                assert {"atomic_load", "atomic_xsub"} <= observed
            else:
                assert all(r["status"] == "ok" for r in functions.values())
print("PASS: default behavior, multi-gap continuation, intrinsic inventory, complete reports and no survey objects")
