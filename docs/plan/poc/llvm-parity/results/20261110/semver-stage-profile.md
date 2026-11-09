# Final integrated semver exact stage profile

Separate checker0 profiling runs; instruction counts are current-thread user instructions. Whole-process cost is reported separately.

| stage | c12-phi G | integrated G | reduction |
|---|---:|---:|---:|
| x86 | 0.7486 | 0.3484 | 53.5% |
| post_inline_optimize_s | 0.8891 | 0.2020 | 77.3% |
| inline_s | 0.6592 | 0.2013 | 69.5% |
| layout | 0.2883 | 0.1789 | 38.0% |
| optimize_s | 0.6019 | 0.0987 | 83.6% |
| verify_l | 0.1521 | 0.0623 | 59.0% |
| build | 0.0908 | 0.0562 | 38.1% |
| verify_s_input | 0.1021 | 0.0443 | 56.6% |
| verify_s_output | 0.1043 | 0.0423 | 59.5% |
| prepare_r | 0.0689 | 0.0295 | 57.3% |
| specialize_r | 0.0232 | 0.0106 | 54.3% |
| object | 0.0091 | 0.0077 | 14.7% |
| identity_s | 0.0723 | 0.0051 | 93.0% |
| permissions_s | 0.0026 | 0.0026 | 0.7% |
| post_inline_identity_s | 0.1570 | 0.0014 | 99.1% |

Stage-window sums may omit work outside instrumented stages. Exact total codegen_crate→join_codegen: c12-phi4.150361974G; integrated1.450426707G; LLVM1.668639394G.
