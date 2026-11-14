# Pinned upstream compiletest

`compiletest.py` builds and invokes Rust's original `compiletest`, pinned to
`e408947bfd200af42db322daf0fadfe7e26d3bd1` (the Rust 1.94.1 toolchain commit).
The source archive SHA-256 is
`063f7b364a2f0dee6d8f6733486ea7b6510c79682129448b2e6bd4ffc3e2a8fe`.

The official tool owns directive interpretation, auxiliary builds, revision
discovery, expected-output normalization and comparison, ignore/needs logic,
runtime error/forbid patterns, and test verdicts. The local protocol parser is a
planning and explicit-blocking layer; it does not replace that execution.

## Build the original tool in E2B

The tool is an ordinary binary and library, without a private `libtest` feature.
Its original manifest has only two path dependencies: `build_helper` and
`miropt-test-tools`. `build_helper` also includes the original `src/stage0` at
compile time. The preparation helper copies those source trees, the original
lockfile, minicore source, and licenses from the checked archive into their
original relative paths. It generates only a small workspace-root `Cargo.toml`.
`source-provenance.json` records every copied file's hash; the build refuses
modified inputs.

Run compilation on the configured E2B host, with its ordinary LLVM host
toolchain. Do not build it using the experimental backend or on the local
ENOSPC workstation.

```bash
python3 scripts/rustc-upstream/compiletest.py prepare \
  --archive /tmp/kfrc-rust-e408947bfd200af42db322daf0fadfe7e26d3bd1.tar.gz \
  --output /tmp/official-compiletest
python3 scripts/rustc-upstream/compiletest.py build \
  --workspace /tmp/official-compiletest \
  --target-dir /tmp/official-compiletest-target \
  --toolchain 1.94.1 --jobs 4
```

The original 643-package lockfile also contains the rest of Rust's workspace.
An E2B build confirmed that Cargo refuses `--locked` after reducing that
workspace to three members. The helper retains the original byte-exact lock as
`build-evidence/upstream.Cargo.lock`, starts from it, and runs `cargo +1.94.1
update --workspace` to derive the smaller workspace lock. It rejects any
registry package whose name, version, source, or checksum is absent from the
original lock. It saves that precise generated lock as `workspace.Cargo.lock`
and then builds with `cargo +1.94.1 build --locked --release -p compiletest
--bin compiletest`. This changes a generated lock artifact, preserving the
original dependency resolutions and all tool source bytes.

Raw lock-update/build stdout and stderr, both commands, return codes, binary
hash, package counts, and both lock hashes are retained in `build-evidence/`.
Previous attempts are copied into numbered subdirectories before a retry.
Initial source hashes cover the retained original lockfile too. Python 3.11 or
newer is required for the standard-library TOML parser used to validate locks.

## Official CLI and Python API

`source_root` must contain `tests/ui/`: for imported tests this is the vendor
root, rather than `vendor/tests` or `vendor/tests/ui`. `build_root` is separate
for each arm and invocation. The `source` argument is the manifest path such as
`ui/binop/binop.rs`. Supply the real compiler returned by `rustup which
--toolchain 1.94.1 rustc`; the probe checks its commit hash and obtains its
sysroot, host and LLVM version.

The source root also needs the original files below, even for a single exact
UI test. The tool computes a common-input timestamp before filtering tests and
unconditionally traverses `src/etc/natvis/`, `src/tools/run-make-support/`, and
`src/tools/compiletest/` (4, 32, and 55 upstream files). The importer preserves
those complete directories as support resources. It also retains the seven
pretty-printer sources in `src/etc/` named by `common_inputs_stamp`
(`rust_types.py`, `gdb_load_rust_pretty_printers.py`, `gdb_lookup.py`,
`gdb_providers.py`, `lldb_batchmode.py`, `lldb_lookup.py`, `lldb_providers.py`)
and `tests/auxiliary/minicore.rs`. Empty substitute directories would hide
missing original inputs. Rustdoc-only resources are outside this UI invocation.

```python
toolchain = compiletest.probe_toolchain(rustc_path)
result = compiletest.invoke(
    compiletest_binary, toolchain, vendor_root, arm_build_root,
    "ui/binop/binop.rs", logs=arm_logs,
    backend=backend_dylib_or_none,
    rustc_flags=("-Cpanic=abort", "-Csave-temps=yes", "-Zmir-opt-level=0"),
    runner=capture_runner,
    minicore=tool_workspace / "tests/auxiliary/minicore.rs",
    env_overrides={
        "KFRC_UPSTREAM_CAPTURE_ROOT": str(arm_logs / "runs"),
        "KFRC_UPSTREAM_BUILD_ROOT": str(arm_build_root),
    },
)
```

`command(...)` has the same arguments except `logs` and `env_overrides`, and
returns the argv list without executing. `invoke(...)` returns the command,
return code, raw log paths, and original official JSON `events`. It also writes
`compiletest.stdout`, `compiletest.stderr`, `events.json`, and
`invocation.json`. It sets `RUSTC_BOOTSTRAP=1`, `RUST_TEST_THREADS=1`, and
`NO_COLOR=1`. The original runtime and compiler behavior still determines the
official verdict.

The CLI equivalent accepts `run --tool ... --rustc ... --source-root ...
--build-root ... --source ui/...rs --logs ...`, optional `--backend`, `--runner`
and `--minicore`, and repeated `--rustc-flag=-Cpanic=abort` options. Environment
overrides can be supplied in the invoking environment.

The adapter supplies every required option from the pinned `parse_config`:
`compile-lib-path`, `run-lib-path`, `rustc-path`, `python`, `src-root`,
`src-test-suite-root`, `build-root`, `build-test-suite-root`, `sysroot-base`,
`stage`, `stage-id`, `mode`, `suite`, `cc`, `cxx`, `cflags`, `cxxflags`,
`llvm-components`, `channel`, `nightly-branch`, `git-merge-commit-email`,
`minicore-path`, and `jobs`. It also supplies `host` and `target`, which are
optional to getopts but required when parsing the config. Native x86-64 is the
supported target; `llvm-components=x86` advertises only that component. The
channel reflects the compiler release, rather than pretending it is nightly.

The kfrc arm uses the official `--override-codegen-backend=<absolute dylib>`
option, which reaches all normal and auxiliary `make_compile_args` calls. Each
explicit compiler flag is passed through both `--host-rustcflags` and
`--target-rustcflags`. Source directives can override generic compiler flags;
the planning layer must explicitly block conflicts with the selected PoC
configuration instead of editing source directives.

The first complete batch exposed a separate compatibility limit for auxiliary
crates. On native Linux, `build_auxiliary` normally appends `--crate-type dylib`
after all generic and source flags. Under this PoC's `panic=abort` and static
standard-library profile, that dylib already incorporates `std` and the panic
runtime; linking the main crate then fails the single-copy dependency rule and
can combine `panic_unwind` with `panic_abort`. A global `--crate-type rlib` flag
does not solve this: it also changes the primary crate, and the later official
auxiliary `dylib` option remains. The planner explicitly blocks this original
protocol/profile combination instead of rewriting the auxiliary source.

The exceptions follow the pinned tool, rather than the primary test's flags:

- `aux-bin` is explicitly a binary. `proc-macro` is explicitly a proc-macro and
  forces the host target; proc-macro execution remains outside this PoC profile.
- An auxiliary's own, active `no-prefer-dynamic` directive suppresses that
  appended crate-type. Its original attributes and compile flags then determine
  the output format. This exception is usable only with a static `lib`/`rlib`
  format in this profile for `aux-build`, whose crate is found through `-L`.
  The original no-prefer-dynamic branch still returns `AuxType::Dylib`:
  `aux-crate` consequently constructs an explicit `--extern=.../libname.so`
  path, even when the original auxiliary emits an rlib. That combination is
  also blocked; the adapter does not invent a `.so` file or change its type.
  The main file's directive is not inherited; the
  official `from_aux_file` copies only incremental configuration and sets
  `ignore_pass` before reading the auxiliary's directives for the same revision.
- Without the auxiliary directive, an explicit source `crate_type="rlib"` or
  source compile flag does not prevent the final official `dylib` argument.
  `force-host` does not change this choice on a native Linux host.
- Targets without dynamic linking receive `--crate-type lib`; native Linux
  reports dynamic linking support. `needs-dynamic-linking` is a capability
  check, and does not change the target or the chosen auxiliary format.

Apply these checks to transitive auxiliaries too. The pinned tool resolves
their paths from the primary test's adjacent `auxiliary/` directory and uses
the same active revision. Batch-one's original two LLVM failures remain in
their recorded results; a later plan reports the explicit incompatibility.

## Test identity, filtering, and stopping

The pinned tool has no `--list` or `--revision` switch. Its official JSON
stream contains `suite/started`, `test/started`, terminal `test/ok`,
`test/failed` or `test/ignored`, and a terminal suite event. The `name` is
`[ui] tests/ui/family/file.rs#revision`, with `#revision` omitted for a test
without revisions. Planning an inventory is not an execution result: use the
actual started and terminal events to record what the official tool ran.

Each `invoke` has a 120-second timeout, configurable with `timeout=` or CLI
`--timeout`. The official tool starts in a fresh process group; a timeout kills
that group, keeps partial logs/events, and returns `timed_out=true`. This is an
execution failure, rather than an official test pass or an ignored test.

Without `--exact`, positional filters and `--skip` values use substring matches
against the displayed name. With `--exact`, both compare the complete path
relative to `tests/ui`, such as `binop/binop.rs`; the revision suffix is not in
that filterable path. Therefore the helper runs one exact **file with all its
revisions**, and outer results retain one official TestName per revision. A
single revision cannot be selected with a revision-suffixed `--exact` filter.
Multiple positional filters are ORed; repeated `--skip` filters exclude matches.

The helper uses `--force-rerun --exact --fail-fast`. `--force-rerun` prevents a
stamp cache hit from becoming apparent execution. `--fail-fast` stops after the
first failed completion, and `RUST_TEST_THREADS=1` prevents other revisions
starting concurrently. `--jobs=1` alone does not set executor concurrency. The
outer driver handles stopping later files of the same family and marking
unstarted tests explicitly. `fail_fast=False` or CLI `--no-fail-fast` disables
only the official per-invocation stopping behavior.

## Runtime outputs and exit status

Original source sidecars are named
`file[.revision][.compare-mode].run.stdout` and `.run.stderr`. The official tool
compares them only with `//@ check-run-results`; ordinary run-pass output is
still available to the capture runner. Compiler stdout/stderr comparisons,
error patterns, forbid-output, revisions, and all other supported directives
remain under the official implementation.

For UI mode the output directory is
`<build_root>/ui/<relative parent>/<stem>[.<revision>]/`, and the executable is
`a` on Linux. Compare modes add another directory suffix when used. Its raw
`<stem>[.<revision>].out` and `.err` files are overwritten by each subprocess
that `compose_and_run` executes. They cannot unconditionally be labeled runtime
output: a compiler failure can leave compiler output there. The outer capture
runner records `runs/run-*/stdout`, `stderr`, and `run.json` with execution
status for each actual runtime process under its capture root, forwarding the original bytes and
preserving signal termination. It runs through the tool's official `--runner`
option; it does not run the program a second time. The runner path must contain
no spaces because the official tool splits the runner string on spaces.

The original UI runtime rule classifies code 0 as pass, codes 1 through 127 as
failure, and signals or other codes as crash. `run-fail` accepts a failure;
it also accepts a crash only when the **target** cannot unwind. Passing
`-Cpanic=abort` on an unwind-capable Linux target does not change that target
capability. `run-crash` and `run-fail-or-crash` retain their official meanings.
The outer driver must not replace these rules with a hardcoded exit code 101.
An explicit `failure-status` is checked on the compiler-failure path in this
revision; it does not change the UI runtime classification above.

Contract sources in the pinned archive: `src/tools/compiletest/src/lib.rs`
(`parse_config`, `make_test_name_and_filterable_path`), `executor.rs`
(`filter_tests`, `get_concurrency`), `executor/json.rs`, `common.rs`
(`expected_output_path`, `output_base_dir`), `runtest.rs`
(`compose_and_run`, `make_run_args`), and `runtest/ui.rs` (`run_ui_test`).
