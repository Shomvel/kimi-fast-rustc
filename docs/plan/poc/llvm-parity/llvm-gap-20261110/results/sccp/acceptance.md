# SCCP acceptance (before measurements)

Baseline f321983, frozen source.json candidate; one Linux sandbox and shared sysroot.
Correctness before timing: workspace, three-arm semantic/pressure differential,
scalar/generated, corpus/upstream checks with explicit existing unsupported cases.
11 serial alternating runtime rounds, one warmup, separate perf-stat samples.
Three serial alternating compile-cost rounds with checker disabled and every
measured object matching its own reference. A >3% per-workload paired median
slowdown requires independent confirmation and code/layout investigation; do
not adopt confirmed >3% runtime regressions. Smaller differences are not proof
of equality. Report compiler work separately and reject disproportionate overhead
without real benefit. Keep every workload and all failed candidate evidence.
