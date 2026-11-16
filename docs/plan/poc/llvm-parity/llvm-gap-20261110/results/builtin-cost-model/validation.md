# Built-in cost-model validation

Backend SHA-256: `fcb4dc20d17e2ab3004a858c8b0e25ad98f73da311605bea26581fcb2fa341ad`.

Linux workspace: **831 passed, 1 ignored**. Default and observe mode produced byte-identical objects to the frozen baseline in all four libraries. All 5707 observe decision records and 4319 size decision records parsed as schema 1 with complete budget fields.

Explicit size mode matched LLVM for four corpora and all 13 workload checksums. Supported integration suites passed: semver 34, base64 13, unicode-segmentation 8. Existing upstream unit gaps remain recorded in summary.json. The real size-mode semantic test matched 824 lines across three arms; register-pressure differential testing matched 2048 vectors across three arms. Trace evidence confirms size mode (1118 semantic and 121 pressure records).

Size mode is uncalibrated: 3515/4319 decisions are UnknownCost; no real-corpus Cost rejection occurred. The directed unit test covers Cost. Runtime speed of size mode was not measured.

| Library | Default/baseline instructions | Wall ratio | RSS ratio |
|---|---:|---:|---:|
| semver | 1.006968 | 1.029367 | 1.000377 |
| base64 | 1.003226 | 0.982996 | 1.000250 |
| unicode-segmentation | 1.002678 | 0.955877 | 0.998869 |
| humantime | 1.008884 | 0.976018 | 1.003081 |

Three serial alternating rounds; each measured output was byte-identical to baseline. Instruction overhead was +0.27% to +0.89%. Wall/RSS ratios are three-run noisy observations, not speedup evidence.

The first size-mode humantime run exposed a Layout dominance error. Resolving forwarded control/effect aliases fixed the regression; the original reproducer and the full suite passed after that fix. The failed first build remains in the evidence archive.

Old address-sharing/shift v1 and v2 were rejected for runtime regressions and are excluded from the final backend.
