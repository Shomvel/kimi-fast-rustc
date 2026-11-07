#!/usr/bin/env bash
set -euo pipefail

ROOT=${KFRC_BENCH_ROOT:-"$HOME/kimi-fast-rustc-benchmarks"}
KFRC=${KFRC_BIN:-"$HOME/kimi-fast-rustc/target/release/kfrc"}
RUNS=${KFRC_RUNS:-15}
OUT=${KFRC_OUT:-"$ROOT/results/$(date -u +%Y%m%dT%H%M%SZ)"}
CARGO_HOME=${CARGO_HOME:-"$HOME/.cargo"}
CPU_BUDGET=${CARGO_BUILD_JOBS:-4}
PRODUCT_PROFILE=${KFRC_PRODUCT_PROFILE:-balanced}
WARM=${KFRC_WARM:-0}
TARGET=x86_64-unknown-linux-gnu
ARMS=(baseline controlled candidate)
mkdir -p "$OUT"

case $RUNS in
    ''|0|*[!0-9]*)
        echo "KFRC_RUNS must be a positive integer: $RUNS" >&2
        exit 2
        ;;
esac

if [[ ! -x "$KFRC" ]]; then
    echo "kfrc binary not found: $KFRC" >&2
    exit 2
fi

unset RUSTC_WRAPPER RUSTC_WORKSPACE_WRAPPER
unset CARGO_BUILD_RUSTC_WRAPPER CARGO_BUILD_RUSTC_WORKSPACE_WRAPPER
unset KFRC_AUDIT
RUSTC=$(command -v rustc)
CARGO=$(command -v cargo)
export RUSTC CARGO_HOME

"$RUSTC" -Vv >"$OUT/rustc.txt"
"$CARGO" -Vv >"$OUT/cargo-toolchain.txt"
"$KFRC" --version >"$OUT/kfrc.txt"
"$KFRC" fingerprint >"$OUT/system.json"
uname -a >"$OUT/uname.txt"
printf '%s\n' "$RUNS" >"$OUT/runs.txt"
{
    printf 'root=%s\n' "$ROOT"
    printf 'kfrc=%s\n' "$KFRC"
    printf 'rustc=%s\n' "$RUSTC"
    printf 'cargo=%s\n' "$CARGO"
    printf 'cargo_home=%s\n' "$CARGO_HOME"
    printf 'cargo_build_jobs=%s\n' "$CPU_BUDGET"
    printf 'target=%s\n' "$TARGET"
    printf 'candidate_profile=%s\n' "$PRODUCT_PROFILE"
    printf 'warm=%s\n' "$WARM"
    printf 'runs=%s\n' "$RUNS"
} >"$OUT/environment.txt"

build_project() {
    local project=$1
    local package=$2
    local binary=$3
    local source="$ROOT/$project"
    local commit
    local lock_hash
    local variant
    commit=$(git -C "$source" rev-parse HEAD)
    lock_hash=$(sha256sum "$source/Cargo.lock")
    lock_hash=${lock_hash%% *}
    printf '%s\n' "$commit" >"$OUT/$project.commit"
    printf '%s\n' "$lock_hash" >"$OUT/$project.lock.sha256"

    local -a fetch_command=(
        "$CARGO" fetch --manifest-path "$source/Cargo.toml" --locked --offline
        --target "$TARGET"
    )
    {
        printf 'CARGO_HOME=%q RUSTC=%q ' "$CARGO_HOME" "$RUSTC"
        printf '%q ' "${fetch_command[@]}"
        printf '\n'
    } >"$OUT/$project.fetch.command"
    CARGO_HOME="$CARGO_HOME" RUSTC="$RUSTC" \
        "${fetch_command[@]}" >"$OUT/$project.fetch.log" 2>&1

    for variant in "${ARMS[@]}"; do
        local target="$ROOT/target-$project-$variant"
        local log="$OUT/$project-$variant.build.log"
        local timing="$OUT/$project-$variant.time"
        if [[ $WARM != 1 ]]; then
            rm -rf "$target"
        fi
        local -a environment=(
            "CARGO_TARGET_DIR=$target"
            "CARGO_BUILD_JOBS=$CPU_BUDGET"
            "CARGO_HOME=$CARGO_HOME"
            "RUSTC=$RUSTC"
            "KFRC_BACKEND=llvm"
        )
        case $variant in
            baseline)
                ;;
            controlled)
                local audit="$OUT/$project-$variant.audit"
                mkdir -p "$audit"
                environment+=(
                    "RUSTC_WRAPPER=$KFRC"
                    "KFRC_PROFILE=controlled"
                    "KFRC_AUDIT=$audit"
                )
                ;;
            candidate)
                local audit="$OUT/$project-$variant.audit"
                mkdir -p "$audit"
                environment+=(
                    "RUSTC_WRAPPER=$KFRC"
                    "KFRC_PROFILE=$PRODUCT_PROFILE"
                    "KFRC_AUDIT=$audit"
                )
                ;;
        esac
        printf '%s\n' "${environment[@]}" >"$OUT/$project-$variant.environment"

        local -a command=(
            "$CARGO" build --manifest-path "$source/Cargo.toml"
            --release --offline --locked --package "$package" --bin "$binary"
            --target "$TARGET"
        )
        local -a timed_command=(
            python3 -c '
import resource
import subprocess
import sys
import time

start = time.perf_counter()
completed = subprocess.run(sys.argv[2:])
elapsed = time.perf_counter() - start
usage = resource.getrusage(resource.RUSAGE_CHILDREN)
with open(sys.argv[1], "w") as stream:
    stream.write(
        f"wall={elapsed:.2f} user={usage.ru_utime:.2f} "
        f"sys={usage.ru_stime:.2f} maxrss={usage.ru_maxrss}\n"
    )
sys.exit(completed.returncode)
' "$timing"
            env "${environment[@]}" "${command[@]}"
        )
        {
            printf '%q ' "${timed_command[@]}"
            printf '>%q 2>&1\n' "$log"
        } >"$OUT/$project-$variant.command"
        echo "building $project/$variant"
        "${timed_command[@]}" >"$log" 2>&1
        cp "$target/$TARGET/release/$binary" "$OUT/$project-$variant"
    done
}

build_project ripgrep ripgrep rg
build_project cargo cargo cargo

KFRC_BENCH_ROOT="$ROOT" \
KFRC_BENCH_OUT="$OUT" \
KFRC_BENCH_RUNS="$RUNS" \
KFRC_BENCH_WARM="$WARM" \
KFRC_BENCH_PRODUCT_PROFILE="$PRODUCT_PROFILE" \
python3 - <<'PY'
import json
import os
import statistics
import subprocess
import time

root = os.environ["KFRC_BENCH_ROOT"]
out = os.environ["KFRC_BENCH_OUT"]
runs = int(os.environ["KFRC_BENCH_RUNS"])
warm = os.environ["KFRC_BENCH_WARM"] == "1"
product_profile = os.environ["KFRC_BENCH_PRODUCT_PROFILE"]
arms = ("baseline", "controlled", "candidate")

workloads = {
    "ripgrep": lambda binary: [
        binary, "-n", "--no-messages", r"fn\s+main", os.path.join(root, "ripgrep")
    ],
    "cargo": lambda binary: [
        binary, "metadata", "--format-version", "1", "--no-deps", "--offline", "--locked",
        "--manifest-path", os.path.join(root, "cargo", "Cargo.toml"),
    ],
}


def read_text(path):
    with open(path) as stream:
        return stream.read().strip()


def build_metadata(project, arm):
    prefix = os.path.join(out, f"{project}-{arm}")
    fields = dict(field.split("=", 1) for field in read_text(prefix + ".time").split())
    return {
        "command": read_text(prefix + ".command"),
        "environment": read_text(prefix + ".environment").splitlines(),
        "compile_time": {
            "wall_seconds": float(fields["wall"]),
            "user_seconds": float(fields["user"]),
            "system_seconds": float(fields["sys"]),
            "max_rss_kibibytes": int(fields["maxrss"]),
        },
    }


for project, command_for in workloads.items():
    paths = {arm: os.path.join(out, f"{project}-{arm}") for arm in arms}
    for path in paths.values():
        os.chmod(path, 0o755)
    commands = {arm: command_for(path) for arm, path in paths.items()}
    samples = {arm: [] for arm in arms}
    for arm in arms:
        subprocess.run(commands[arm], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, check=True)
    for _ in range(runs):
        for arm in arms:
            start = time.perf_counter_ns()
            subprocess.run(commands[arm], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, check=True)
            samples[arm].append((time.perf_counter_ns() - start) / 1e9)
    result = {
        "project": project,
        "commit": read_text(os.path.join(out, f"{project}.commit")),
        "lockfile_sha256": read_text(os.path.join(out, f"{project}.lock.sha256")),
        "runs": runs,
        "warm": warm,
        "candidate_profile": product_profile,
        "commands": commands,
        "builds": {arm: build_metadata(project, arm) for arm in arms},
        "variants": {},
    }
    for arm in arms:
        values = samples[arm]
        result["variants"][arm] = {
            "binary_bytes": os.path.getsize(paths[arm]),
            "samples_seconds": values,
            "median_seconds": statistics.median(values),
            "mean_seconds": statistics.fmean(values),
            "min_seconds": min(values),
            "max_seconds": max(values),
            "stdev_seconds": statistics.stdev(values) if len(values) > 1 else 0.0,
        }
    baseline = result["variants"]["baseline"]
    for arm in ("controlled", "candidate"):
        variant = result["variants"][arm]
        result[f"{arm}_over_baseline"] = {
            "binary_size": variant["binary_bytes"] / baseline["binary_bytes"],
            "median_runtime": variant["median_seconds"] / baseline["median_seconds"],
        }
    with open(os.path.join(out, f"{project}.json"), "w") as stream:
        json.dump(result, stream, indent=2)
        stream.write("\n")
    print(json.dumps({
        "project": project,
        "controlled_over_baseline": result["controlled_over_baseline"],
        "candidate_over_baseline": result["candidate_over_baseline"],
    }, sort_keys=True))
PY

echo "results: $OUT"
