"""Line-based delta reduction of one mismatching generated function.

Repeatedly deletes single lines (and balanced line ranges) of the target
function's body, keeping a deletion when the program still builds with both
backends and the kfrc and LLVM results still differ.

usage: python3 -I reduce.py <lib.rs> <function> <args tuple> <backend .so> <work dir>
"""

import os
import re
import subprocess
import sys

FLAGS = ["--edition", "2021", "-Cpanic=abort", "-Coverflow-checks=off", "-Copt-level=0",
         "-Cdebug-assertions=off", "-Zmir-opt-level=0", "-Awarnings"]


def run(command, **kwargs):
    return subprocess.run(command, capture_output=True, text=True, **kwargs)


def outcome(source, function, args, backend, work):
    os.makedirs(work, exist_ok=True)
    lib = os.path.join(work, "lib.rs")
    with open(lib, "w") as handle:
        handle.write(source)
    driver = os.path.join(work, "driver.rs")
    with open(driver, "w") as handle:
        handle.write(f"fn main() {{ println!(\"{{:?}}\", generated::{function}{args}); }}\n")
    env = dict(os.environ, RUSTC_BOOTSTRAP="1")
    results = {}
    for arm, extra in (("llvm", []), ("kfrc", [f"-Zcodegen-backend={backend}"])):
        out = os.path.join(work, arm)
        os.makedirs(out, exist_ok=True)
        built = run(["rustc", "+1.94.1", *FLAGS, "--crate-type", "rlib", "--crate-name", "generated", lib, "--out-dir", out, *extra], env=env)
        if built.returncode != 0:
            return None
        binary = os.path.join(out, "driver")
        linked = run(["rustc", "+1.94.1", *FLAGS, "--crate-name", "driver", driver,
                      "--extern", f"generated={out}/libgenerated.rlib", "-o", binary], env=env)
        if linked.returncode != 0:
            return None
        try:
            executed = run([binary], timeout=10)
        except subprocess.TimeoutExpired:
            results[arm] = "timeout"
            continue
        results[arm] = executed.stdout + f"exit={executed.returncode}"
    return results


def main():
    path, function, args, backend, work = sys.argv[1:6]
    source = open(path).read()
    lines = source.split("\n")
    start = next(i for i, line in enumerate(lines) if re.match(rf"pub fn {function}\(", line))
    end = next(i for i in range(start, len(lines)) if lines[i] == "}")
    body = lines[start + 1:end - 1]  # keep the tail expression line

    def build(candidate):
        return "\n".join(lines[:start + 1] + candidate + lines[end - 1:])

    def interesting(candidate):
        results = outcome(build(candidate), function, args, backend, work)
        return results is not None and results["llvm"] != results["kfrc"], results

    ok, results = interesting(body)
    assert ok, f"not reproducible: {results}"
    changed = True
    while changed:
        changed = False
        for size in (8, 4, 2, 1):
            index = 0
            while index < len(body):
                candidate = body[:index] + body[index + size:]
                ok, _ = interesting(candidate)
                if ok:
                    body = candidate
                    changed = True
                else:
                    index += 1
    print(build(body).split("\n")[start - 1:start + len(body) + 3] and "\n".join(lines[start:start + 1] + body + lines[end - 1:end + 1]))
    print(interesting(body)[1])


if __name__ == "__main__":
    main()
