# InstCombine candidate acceptance

Frozen before runtime measurements: compare e3c6295 baseline with the two-file
source.json candidate in one sandbox. LLVM is a reference arm. All correctness
checks must pass before timing. Use 11 serial alternating rounds per workload,
with a separate perf-stat run. Report every paired workload ratio, geometric
mean, object text size and three-round compiler instructions.

A workload with paired median slowdown above 3% triggers a separate confirmation
run and instruction/layout comparison; do not adopt a confirmed regression above
that bound. This is a practical regression screen, not an assertion that smaller
changes are statistically zero. Require a measurable improvement in generated
code, compiler work, or runtime before claiming the candidate as an optimization.
Do not select only favorable workloads or conflate bytes/instructions with time.
