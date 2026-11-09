# semver 1.0.28 differential

Candidate 19 repeated the full differential run in Linux/E2B on
2026-11-09: all **34 unchanged upstream integration tests passed in both arms**,
and the visible workload outputs matched exactly. All 25 published source
files retained their hashes before and after execution. The kfrc
`Version::Debug::fmt` object and linked-driver provenance checks also passed.
This verifies the selected semver library, with the LLVM harness/std and
cross-crate generic limitations described below. It does not establish a
performance result.

The final candidate 19
[`semver/summary.json`](../../../../evidence/poc-20261109/candidate19-results/results/candidate-19/semver/summary.json)
has SHA-256 `c3e1fdba91fb68c350f60263d8ed6f53a7b9e9f0eecc592d4b5ace594ca43a07`.
The final backend hash is
`b16007417002cdea3a55e465e0d0cb0febddcc7485ebc38e9a86bd95a814ecb4`;
the compiler source archive hash is
`0357d1d76dff113acd8ff67ab8e817ea64b8152b0f4acaac46f04fd268b0a492`.
[`final-provenance.json`](../../../../evidence/poc-20261109/candidate19-results/results/candidate-19/final-provenance.json)
records the final source, backend and tool identities.

The first successful candidate 15 patch 02 result is retained separately in
[`summary.json`](../../../../evidence/poc-20261109/patch02-call15-results/results/candidate-15/patch-02/semver/summary.json),
SHA-256 `8a091dcd17d6152fc9dec30767d7d8be13051b82ac5dd5b82c51549fa6d9aad5`.
The backend SHA-256 was
`c8160ed11d59b123d71afdb750285651ae345ca8777ad0e825dccb783ba4a03f`;
the compiler source archive SHA-256 was
`fc4437efc03a4e732a1a45eb87fbac14b17b316b1362cc39f6cef522ceaf129b`.
Both workload files have 27 lines and SHA-256
`d3da705ee02c167e5831feb21eb71b9b9efd4a8076d0ad309fa7d2baea899354`.

```sh
bash crates/rustc_codegen_kfrc/tests/p3-semver/run.sh /absolute/path/to/librustc_codegen_kfrc.so /fresh/results/semver
```

The runner downloads the exact published crate and verifies its crates.io SHA-256
(`8a7852d02fc848982e0c167ef163aaff9cd91dc640ba85e263cb1ce46fae51cd`).
An existing `semver-1.0.28.crate` in the result directory is accepted only after
that same check. The source directory must be new; files are not patched.
This version is also the one locked by the existing Cargo benchmark checkout.

Cargo selects the backend only for the semver package. The workload binary,
upstream integration-test code, libtest, and std use LLVM. Four unchanged
upstream integration suites contain 34 tests with default `std` features. There
are no source unit tests or `should_panic` tests in this release. Optional serde,
Criterion benchmarks, doc tests, and the separately configured Node.js oracle
are outside this command's scope. `-Cpanic=abort -Zpanic-abort-tests` is used for
the LLVM test harness to link the abort library with libtest.

Both arms use `-Zmir-opt-level=0`, opt-level 0, and disabled overflow/debug checks.
The comparison first requires successful process exits, then exact workload
output and identical passing test identities. Raw JSON test events, build logs,
compiler versions, checksums, and source hashes are retained. The kfrc arm also
requires ELF `FILE kfrc` provenance in the semver rlib. This is a correctness
milestone; it does not establish the PoC performance or cache claims.

The runner checks that the published manifest declares exactly these four
integration suites and no source unit tests, then verifies every extracted
source hash again after execution. Visible workload output includes successful
Version and VersionReq Debug formatting. `debug-provenance.json` requires the
non-inline `Version::Debug::fmt` definition to live in a `FILE kfrc` object from
the semver rlib and requires the same symbol in the linked driver. VersionReq's
derived inline Debug may be instantiated by the LLVM driver; its visible output
alone does not prove a kfrc method definition. These checks passed in candidate
15 patch 02 and again in candidate 19 along with the complete upstream
integration suite.

`dyn-probe.sh <backend.so> <fresh-results>` isolates the trait-object frontier:
manual Debug, derive Debug with three and six fields, virtual formatting of an
LLVM-created trait object, and a capturing closure. The library uses the
selected backend and the driver uses LLVM. It compares visible output and
requires kfrc object provenance. This probe does not replace the upstream tests.
Candidate 11 passed all 12 probe output lines, including the real Debug callback
and virtual dispatch paths.

`regex-smoke.sh <backend.so> <another-fresh-results>`
provides a separate public-API frontier for fixed regex-syntax 0.8.8 with its
default `std` and Unicode features. It compares 18 patterns, including valid
Unicode expressions and parse errors, and checks the library's object
provenance. It does not run or replace regex-syntax's internal unit-test suite.
Candidate 19 passed this smoke comparison. Its 16 kfrc codegen units also
contained 8,584 private allocations, all with LOCAL binding and no duplicate
GLOBAL data symbols. This checks data ownership across codegen units; it is
separate from the smoke workload's behavior comparison.
The two 30-line workload files have SHA-256
`f78807c961ef877c20b77dd0cc149d70733ee2cb54d3e00f494460a7e78a5c84`;
the object audit is retained as
[`data-binding-verification.json`](../../../../evidence/poc-20261109/candidate19-results/results/candidate-19/regex-smoke/kfrc/objects/data-binding-verification.json),
SHA-256 `b5ce5e88eb64c1379b85fda1d6c73290d5225e01ad00e0d1a010ee6a545ca79b`.

The full unchanged internal suite has a separate command:

```sh
bash crates/rustc_codegen_kfrc/tests/p3-semver/regex-tests.sh /absolute/backend.so /fresh/regex-tests
```

This uses stock rustc `--test` on the original `src/lib.rs`, with the exact
default feature closure read from the published manifest. Library code, unit
tests, and the generated harness all use the selected backend; prebuilt
libtest/std use LLVM. It retains temporary objects, compiler commands, every
discovered test name, and JSON outcomes. No tests are extracted, patched or
filtered. `--include-ignored` prevents an ignored test from silently satisfying
the all-tests requirement. Completion requires every name discovered by the
LLVM baseline to be present and pass under kfrc.

The complete LLVM baseline passed **147/147** tests. Candidate 19 did not
compile the full kfrc test binary: SoN bb12 rejected construction of an
uninhabited enum in
`core::array::try_from_fn_erased<NeverShortCircuit<hir::literal::Literal>, Drain<...>>`.
No kfrc internal test was executed, filtered out or replaced to obtain the
public API smoke result. Full-suite support remains unfinished at this
compilation boundary; no complete kfrc 147-test pass is claimed.
The retained
[`regex-full/partial-summary.json`](../../../../evidence/poc-20261109/candidate19-results/results/candidate-19/regex-full/partial-summary.json)
records LLVM 147 passed, zero ignored/filtered and zero kfrc tests executed;
its SHA-256 is `1f24b63b3cc29e6bf96c5fe814138b07e764827161f129433df7914fcbc53b4d`.
The original candidate compiler diagnostic remains in
[`regex-full/kfrc/build.log`](../../../../evidence/poc-20261109/candidate19-results/results/candidate-19/regex-full/kfrc/build.log).

`regex-tests.sh <backend> <work> baseline` prepares only the LLVM reference;
`regex-tests.sh <backend> <same-work> candidate` then uses kfrc. Existing result
arms are never overwritten. A failed candidate remains evidence and needs a
new result directory for another attempt.
