# Benchmark methodology

Formal builds run only on `yicloud-dev` and compare the same rustc, sysroot, source commit, lockfile, target, features, dependency cache, and CPU budget. Baseline and candidate use separate target directories and absolute tool paths. Dependency fetching happens before the timed region; builds use `--offline`.

Three configurations are distinct:

- `R`: stock rustc+LLVM without a wrapper.
- `K-controlled`: kfrc with identical effective rustc arguments, isolating wrapper and scheduling overhead.
- `K-product`: kfrc's explicitly recorded optimization profile.

Portable and `target-cpu=native` builds are separate matrices. LLVM reference results are never labeled native-backend results. Native results require separate correctness tests and proof that no LLVM tool or subprocess participated.

Record full commits, lockfile hashes, Cargo metadata, complete build commands, rustc/LLVM versions, CPU topology, cgroup quota/cpuset, memory, and dependency tool versions. Cold builds clean their own target directories. Warm builds are reported separately. No cache may cross cold-build arms.

Measure binary size and real runtime together with compile time. Runtime workloads include recursive search for ripgrep and offline metadata processing for cargo. Alternate baseline and candidate after warmup, retain raw samples, and report median, mean, standard deviation, minimum, and maximum. Candidate-over-baseline ratios at or below 1.0 indicate non-regression for both size and runtime. A debug or failed build is not formal evidence.

The initial `scripts/bench-yicloud.sh` implements the basic controlled comparison. Whole-project parity is an unmet acceptance criterion until successful formal results exist for both ripgrep and cargo.
