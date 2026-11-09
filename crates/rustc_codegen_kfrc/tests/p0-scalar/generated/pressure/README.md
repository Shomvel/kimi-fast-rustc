# Allocation pressure differential

Runs four functions over 512 deterministic seeds (2,048 output lines per arm),
with LLVM, kfrc optimization disabled, and kfrc optimization enabled. A separately
compiled C provider computes expected values and really clears all caller-saved
GPRs before returning. The corpus covers 24 live values across a clobber and alias
writes, 16-value Phi rotations, division/remainder/variable shifts, and 9 incoming
/ 8 outgoing reordered arguments. Arithmetic is unsigned wrapping; buffers,
shift counts and divisors are bounded. Allocation checking stays enabled.

On x86-64 Linux with Rust 1.94.1 and a built backend:

```sh
python3 generate.py /absolute/output/inputs
python3 run.py --inputs /absolute/output/inputs --backend /absolute/librustc_codegen_kfrc.so --out /absolute/output/results
```

The runner stops at the first failure, records commands and exit codes, and checks
all arm output bytes. `scripts/poc-validation/run-linux.sh <repo> <out> pressure`
provides the repository entry point. This is correctness stress, not a benchmark.
