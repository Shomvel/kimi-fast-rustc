# Storage representation differential

Run on Linux with the pinned rustc 1.94.1:

```sh
bash run.sh /absolute/path/librustc_codegen_kfrc.so /tmp/storage-representation-fresh
```

The library is compiled by LLVM, the original x86 layer, Cranelift with frame
memory, and Cranelift with storage lanes. Every driver is LLVM-compiled and
checks 4096 deterministic input cases against independent byte-array results.
The checks include mixed-width writes, copies crossing word boundaries,
overlapping moves in both directions, partially initialized `MaybeUninit`,
and whole result/parameter transport. Only initialized bytes are observed.

Each command, exit status, source and output hash, toolchain and stdout/stderr
is saved. A mismatch or failed command stops the runner immediately.
