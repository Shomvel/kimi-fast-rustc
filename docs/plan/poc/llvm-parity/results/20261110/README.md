# Final1 integrated E2B acceptance

Frozen final1 is c12 + correctness repairs + array transport + compiler-cost optimizations; return merging deferred. Source SHA `421716c1198dfd95d975a8167b47582dbc4abfce8fd62f338b3a48c1a65d00eb`; backend SHA `4300c5c66ded8dfa21c7b97f61519f7716d9b44801c6198ccddbb2979844c8f4`.

All 11 acceptance stages passed. Unit: 90 suites, 824 passed, 1 ignored. Semantic: three arms, 824 identical lines each. Scalar 565; generated 5×40; pressure 2048; semver upstream 34/34 per arm; seven legacy suites in all three arms; native CLI/features/RS3 shapes/u128 unsupported rejection passed. This run uses the actual `legacy-corpus` stage (the older c12 acceptance's `legacy` stage name was a no-op).

Original unicode-bidi 0.3.18 compiled O3/CGU16/checker1 under LLVM, c12-phi, and integrated. All 768 deterministic reorder maps and permutation assertions matched. The LLVM driver references unicode-bidi library code (nm evidence retained).

Five-crate correctness preflight preceded all performance: checker1 explicitly enabled. Four compilable library corpus outputs and 13 workload checksums matched LLVM, with no failing tests. Semver 34, base64 13, unicode-segmentation 8 upstream integration tests passed in all arms. Full upstream correctness remains incomplete: base64 unit builds expose TLS in test-only RandomState; unicode-segmentation unit requires atomic_xsub; humantime unit requires u128. regex-syntax remains uncompiled: integrated reaches SIMD; c12-phi stops at caller-location adapters. These gaps are recorded as incomplete/compile_failed, never passing tests, and regex is excluded from four-crate averages. See `tls-scope.md` for the newly exposed TLS boundary. LLVM reference unit counts: regex 147 / base64 179 / unicode-segmentation 15 / humantime 38.

Baseline c12-phi is c12 plus the self-Phi register-home correctness guard, backend SHA `4a49d09600bb6006d0f403869213592a48f230686af1f83268201ff98c97ccb4`; it has no compiler-cost or array optimizations. All arms use the same MIR0 sysroot, toolchain, source path, formal O3/CGU16 flags, and sandbox. No unrelated workload ran during formal measurements.

## Formal compile cost

Whole-rustc process, checker0 explicitly selected, three forward/reverse/forward arm orders, median user instructions. Raw samples, wall/RSS and metadata approximation are retained. Initial original-suite checker1 costs remain under `compile_checker1_not_formal`; the generated table uses replacement checker0 formal costs. Every formal repeat matched checker1 reference object bytes.

| crate | c12-phi G | integrated G | reduction | integrated/LLVM | wall base→final s | RSS base→final MiB |
|---|---:|---:|---:|---:|---:|---:|
| semver | 4.537 | 1.837 | 59.5% | 0.894× | 0.594→0.340 | 146.0→144.6 |
| base64 | 4.932 | 1.494 | 69.7% | 0.944× | 0.586→0.290 | 142.2→140.3 |
| unicode-segmentation | 3.366 | 1.835 | 45.5% | 0.856× | 0.467→0.340 | 135.8→134.8 |
| humantime | 10.123 | 2.208 | 78.2% | 1.588× | 1.026→0.356 | 221.7→161.3 |

Four-crate geomean whole-process reduction: 65.3%. The formal compile gate remains unmet (integrated/LLVM geomean 1.035×; threshold 0.33×).

## Separate exact codegen profile

Exact codegen_crate→join_codegen user-instruction window over all initial process threads and their descendants (including LLVM workers), checker0; collected separately after runtime/whole-process measurements. Stage-instrumented objects matched each arm's uninstrumented-profile reference.

| crate | c12-phi G | integrated G | reduction | integrated/LLVM |
|---|---:|---:|---:|---:|
| semver | 4.150 | 1.450 | 65.1% | 0.869× |
| base64 | 4.449 | 1.006 | 77.4% | 0.920× |
| unicode-segmentation | 2.312 | 0.782 | 66.2% | 0.717× |
| humantime | 9.801 | 1.888 | 80.7% | 1.766× |

Four-crate geomean codegen-window reduction: 73.2%. These are the final integrated source's actual measurements, distinct from earlier compile-only runs.

## Size and runtime

Base64 text decreased 54307→29651 bytes (−45.4%); semver 34148, unicode-segmentation 18618, humantime 57477 bytes are unchanged vs c12-phi. Driver objects were identical across all three arms. Eleven alternating paired runtime rounds plus separate perf runs found integrated/c12-phi 13-workload geomean 0.9992×; do not claim a runtime speedup from this near-noise difference. Against LLVM, four-crate text geomean 2.965× and 13-workload runtime geomean 3.443×: size/runtime gates also remain unmet. The full generated per-workload table is [`suite/table.md`](suite/table.md).

## Evidence and retention

All evidence downloaded and hash-verified. Archive SHA `d326f1f054319fedc4da57b064d671dabb17a52f178d3945ca456aac8e92d36e`. Backend exported as `backend.so`. Scripts used are retained beside this report; no credentials included. Root confirmed the main-tree writeback fingerprints. Sandbox `is26rukbch0353h4muarf` was released at `2026-11-09T17:45:45.381813+00:00`; `sandbox-release.json` confirms `killed: true`.

## Published source and evidence

All 1,064 non-document compiler/input files in `source-sha256.json` match the frozen E2B
candidate byte for byte. The original source archive also contained planning documents;
those under `docs/plan/poc/rust-semantics/` remain separate from this implementation PR
(the earlier planning PR is #6). No compiler source changed during publication.

This directory contains compact JSON results and runtime samples, the generated gate table,
profile summaries, source fingerprints, and the sandbox release receipt. Large build artifacts
and full logs are retained locally under the archive digest above. The portable suite sources
and reporting regressions are published under `tools/llvm-parity-suite/`; `tools/compile-cost/`
contains the codegen profiling tools. Formal commands are recorded in `final-commands.tsv`.
