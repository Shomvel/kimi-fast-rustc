# Large-crate capability survey, 2026-11-20

Baseline: `4ccaa863a3cabaa8c87b127a715d34118f35e604` on `perf/runtime-gap-20261117`.
Implementation: `e653303` (basic atomics and diagnostic survey); draft PR #21.

## Inputs and method

- ripgrep 14.1.1, commit `4649aa9700619f94cf9c66876e9549d83420e16c`.
- cargo 0.95.0, commit `85eff7c80277b57f78b11e28d14154ab12fcf643`.
- rustc 1.94.1, commit `e408947bfd200af42db322daf0fadfe7e26d3bd1`, LLVM 21.1.8, x86_64 Linux.
- Isolated E2B: 8 vCPU, 16,016 MiB. Both full release builds first completed with LLVM, including cargo's C dependencies.

Capture successful LLVM rustc invocations with `capture-rustc.py`, then replay each formal target unit using `replay-survey.py`. Formal units have an explicit `--target x86_64-unknown-linux-gnu`, `--crate-name`, and a link-emitting request. Host build scripts, proc macros, rustc print requests and autocfg stdin probes are excluded. There are 33 ripgrep units and 286 cargo units; feature/version duplicates are separate units.

The replay changes the backend and output directory while retaining LLVM dependency metadata and successful build-script outputs. An initial environment allowlist omitted build-script-defined rustc environment variables. A full audit of 319 formal units found 40 retained build-script outputs and exactly three affected units: cargo library/binary (`RUST_HOST_TARGET` and `CARGO_COMMIT_*`) and rg binary (`RIPGREP_BUILD_GIT_HASH`). The replay correction derives declared names/values from the original output, records provenance, and preserves original captures. The other 316 units, including all 14 targeted cargo A/B units, are unaffected. All three affected units have been replayed with the corrected environment. The rg baseline/candidate first-error maps are identical to their original runs. It enables `KFRC_GAP_SURVEY=/path`, `KFRC_MACHINE=cranelift` and one codegen thread. This isolates gaps in downstream units even when a dependency would fail under kfrc.

Each mono function/static/entry records its first failure. The independent MIR intrinsic inventory also sees intrinsic calls hidden behind earlier conversion failures. Intrinsic observations are separate from failures. The diagnostic skips crate-wide transforms/inlining, emits no objects, and always returns failure. Zero survey errors therefore require a separate default-mode build and runtime check. Rustc query/conversion failures remain fatal; only owned graph-pipeline panics are isolated and their graphs discarded. Completion requires a final `summary: complete` record.

## Complete ripgrep results

All 33 reports are complete, covering 35,422 items. First-error items decrease from 1,694 to 1,369 after U1 (325 removed); failing units decrease from 23 to 21. Default-mode independent object generation improves from 10/33 to 12/33, with no pass/fail regressions. Newly successful: `log 0.4.22` and `walkdir 2.5.0`. Full `rg` remains unsupported, including its non-unit executable entry.

In cargo, a targeted default-mode A/B of the 14 units whose observed baseline failures are exclusively covered by U1 improves from 0/14 to 14/14 successful object generation: anstream, arc_swap, bytes, colorchoice, gix_actor, gix_glob, gix_ignore, gix_traverse, libssh2_sys, log, nix, once_cell, sharded_slab and walkdir. These are actual object outputs with hashes, recorded in `cargo-targeted-default-summary.json`; other cargo candidate units and whole-program execution were not checked by this targeted run.

U1 implements signed/unsigned 8/16/32/64-bit atomic load/store/xadd/xsub/exchange/CAS, thin-pointer load/store/exchange/CAS, and fences through crate-private Cranelift helpers. Calls conservatively preserve unknown read/write effects through the semantic optimizer. Weaker memory ordering is strengthened to seqcst, weak CAS uses strong CAS, and compiler fences use a hardware fence. The exact no-arg unit-returning `llvm.x86.sse2.pause` hint is removed, following the pinned cg_clif implementation. Hardware PAUSE emission and performance improvements are not claimed.

The dominant remaining first-error categories are SIMD (548), i128/u128 (241), float semantic/value support (171), union (134), float ABI (83), unreachable control (71), aggregate constants (31), and DST tail layout (30). These categories represent where conversion first failed, rather than complete per-function capability requirements. Floating-point arithmetic/constant needs hidden by ABI/type gates remain unmeasured. See `ripgrep-summary.json` for every category and baseline/candidate counts.

## Native validation

- `crates/rustc_codegen_kfrc/tests/atomics/run.py` builds the same library with LLVM and kfrc and links each into an LLVM driver. Differential output passes for integer widths/signs, wrapping and returned old values, CAS success/failure/weak, pointer operations, fences, four-CGU helper ownership, 80,000 eight-thread RMW increments, 80,000 CAS increments, 200 release/acquire publications and 2,000 seqcst store/load litmus rounds.
- `crates/rustc_codegen_kfrc/tests/gap-survey/run.py` passes six compilation checks: normal supported output, survey supported/no artifact, mixed floating-point and inline-assembly failures, inventory despite conversion failure, full summary, and non-unit main recorded before continuing functions.
- `unlocked-runtime.py` builds entire log/walkdir rlibs using each backend and links LLVM drivers. log installs a logger, sets INFO and emits one INFO record while filtering DEBUG. walkdir traverses a fixed directory tree and compares sorted paths. Both drivers match LLVM output. Link maps assert actual kfrc rlib object-member inclusion, and library/driver hashes are retained.

The full workspace test run passes with `KFRC_VERIFY=full`, `-j2` and one test thread: 1,455 passed, zero failed, one ignored across 117 result groups. The separately compiled backend passes 30 tests across two groups. `test-counts.json` records the counts; `tested-source-commit-match.json` verifies all 18 functional/test files against the exact Linux input. The default-speed gix survey exhausts its 1,800-second unit budget during `remote::connection::fetch::refs::update`. Low-frequency gdb samples identify the kfrc-specific Cranelift copy-cleanup equality analysis; the original process is preserved until its timeout rather than repeatedly restarted. A separate gix survey with the existing `KFRC_CRANELIFT_OPT=none` setting completes all 20,072 items in 1,130 seconds, including 1,049 first failures. Its 11,657 symbols shared with the partial speed run have identical status and first-error messages; all 30 speed-observed panics recur under none (34 in the complete none run). The timeout function completes successfully under none. This is diagnostic coverage evidence, rather than a controlled benchmark. None also encounters a separate slow layout path rebuilding CFG/dominators in `thread_variants`.

The cargo library similarly enters the copy-cleanup slow path under speed, including `sources::git::oxide::amend_authentication_hints`. Its speed run is deliberately stopped after 1,201.3 seconds, with 80,013 completed symbols and no completion marker; this is a manual stop, not a timeout. Its complete none run covers 110,693 symbols in 507.4 seconds. All 80,013 shared symbols have identical status and first-error messages, including all seven speed-observed panics. The last speed function completes successfully under none. The cargo executable none run completes in 16.7 seconds. These timings describe diagnostic runs, not controlled performance comparisons.

## Complete cargo coverage

All 286 formal cargo units have complete reports: 283 speed units and three none units (gix, cargo library and executable). They cover 297,805 items, with 21,254 first failures across 162 units. Across both projects, the baseline covers 333,227 items in 319 complete reports, using 316 speed and three none units. The mixed configuration and default-speed partial reports remain explicit; complete default-speed compilation is not claimed.

Cargo's largest first-error groups are float semantic/value support (7,092), union (5,059), SIMD (2,443), i128/u128 (2,004), float ABI (1,059), unreachable control (761), atomic xsub (581), and aggregate constants (398). The float group contains 6,357 f64 and 586 f32 semantic rejections plus 149 machine value rejections; earlier type/ABI failures mask later arithmetic and constant requirements. There are 98 owned-pipeline panics: 97 inline-input/Phi cases and one CLIF egraph payload assertion. `baseline-classification.md` preserves exact signatures, representative functions, involved crates and intrinsic observations; `summary.json` contains aggregate machine-readable results.

After U1, current first-error sets suggest five cargo C-variadic candidates and five aggregate-constant candidates, plus one ripgrep candidate for each. These are scope estimates, not measured unlocks. Each next unit needs default builds and native runtime checks.

The final raw evidence archive is 21,178,178 bytes, SHA256 `9fd407d89b3bb9bcec3489ef5f1affb4a83849ded63f5c568df75fa1bff0a408`. All evidence is downloaded and checked. The task-owned E2B sandbox has been killed; `sandbox-kill.json` records the result. No peer sandbox is changed.

## Reproduction and artifacts

Build the backend with the pinned toolchain and `rustc-dev`, then run the native regression gates:

```sh
python3 crates/rustc_codegen_kfrc/tests/atomics/run.py /absolute/backend.so /tmp/atomic-gate
python3 crates/rustc_codegen_kfrc/tests/gap-survey/run.py /absolute/backend.so /tmp/survey-gate
```

The capture/replay scripts in this directory document the exact research commands and sandbox root `/workspace/bigcrates`. They require the pinned source checkouts, successful LLVM build outputs, captured invocations and dependency artifacts. Captures and reports include hashes. Use `summarize.py ... --manifest formal-targets.json` to validate completeness against all formal captured inputs, including unstarted units in its denominator and checking each summary against actual item/error counts. `survey-baseline-final-selection-index.json` identifies every selected final report and its configuration.

Raw inputs, detailed per-function reports, full logs and large evidence archives are retained in the task artifact directory `/Users/shom/Downloads/kfrc-bigcrates-20261120/`; they are not bundled into this repository.

`implementation-map.md` maps gaps to kfrc and pinned upstream cg_clif implementations. `units.md` divides follow-up work into 45–90-minute subsets with explicit acceptance checks. Prioritization uses actual whole-crate unlocks where available; float ABI alone does not unlock float-heavy crates because semantic values/operations and i128 remain unsupported.
