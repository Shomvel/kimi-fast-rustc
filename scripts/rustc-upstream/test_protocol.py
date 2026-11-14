"""Tests for planning safety and pinned compiletest parsing semantics."""

import importlib.util
import json
from pathlib import Path
import tempfile
import unittest

SPEC = importlib.util.spec_from_file_location("upstream_protocol", Path(__file__).with_name("protocol.py"))
protocol = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(protocol)


class ProtocolTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.path = self.root / "ui" / "feature" / "test.rs"
        self.path.parent.mkdir(parents=True)

    def parse(self, text, **kwargs):
        self.path.write_text(text, encoding="utf-8")
        return protocol.parse_test(self.path, self.root, "x86_64-unknown-linux-gnu", **kwargs)

    def test_ui_default_is_check_fail(self):
        result = self.parse("fn main() {}\n")[0]
        self.assertEqual((result["mode"], result["expected_failure_status"]), ("check-fail", 1))
        self.assertEqual(result["status"], "ready")
        json.dumps(result)

    def test_revision_selection_and_normalization(self):
        results = self.parse("//@ revisions: Foo-Bar second\n//@[Foo-Bar] run-pass\n//@[second] build-pass\n")
        self.assertEqual([r["mode"] for r in results], ["run-pass", "build-pass"])
        self.assertIn("foo_bar", results[0]["compile_flags"])
        self.assertIn("cfg(test,FALSE,foo_bar,second)", results[0]["compile_flags"])
        chosen = protocol.parse_test(self.path, self.root, "x86_64-unknown-linux-gnu", "second")
        self.assertEqual([r["revision"] for r in chosen], ["second"])

    def test_undeclared_revision_blocks(self):
        self.assertIn("undeclared revision", " ".join(self.parse("//@[other] run-pass\n")[0]["blocked"]))

    def test_duplicate_revision_blocks(self):
        self.assertEqual(self.parse("//@ revisions: a a\n//@ run-pass\n")[0]["status"], "blocked")

    def test_forbidden_and_builtin_revisions_block(self):
        for revision in ("true", "false", "unix"):
            with self.subTest(revision=revision):
                self.assertTrue(self.parse(f"//@ revisions: {revision}\n")[0]["blocked"])

    def test_unknown_directive_blocks_even_in_inactive_revision(self):
        results = self.parse("//@ revisions: a b\n//@[b] future-unknown: value\n")
        self.assertTrue(all(r["blocked"] for r in results))

    def test_known_protocol_is_explicitly_delegated(self):
        result = self.parse("//@ run-pass\n//@ run-rustfix\n//@ needs-unwind\n")[0]
        self.assertEqual(result["status"], "ready")
        self.assertEqual(result["requires"][0]["matched"], None)
        self.assertEqual({d["name"] for d in result["delegated_to_compiletest"]}, {"run-rustfix", "needs-unwind"})

    def test_malformed_selector_and_values_block(self):
        for directive in ("//@[a run-pass", "//@ compile-flags", "//@ edition:", "//@ run-pass build-pass"):
            with self.subTest(directive=directive):
                self.assertTrue(self.parse(directive)[0]["blocked"])

    def test_whitespace_remark_is_safe(self):
        self.assertEqual(self.parse("//@ run-pass     \n")[0]["status"], "ready")

    def test_split_flags_matches_upstream_not_shell(self):
        self.assertEqual(protocol.split_flags("-O '--cfg=hello world' \"two words\" a'b c'd ''"),
                         ["-O", "--cfg=hello world", '"two', 'words"', "a", "b c", "d", ""])
        self.assertEqual(protocol.split_flags("'unclosed string"), ["unclosed string"])

    def test_flag_order_and_edition(self):
        result = self.parse("//@ run-pass\n//@ compile-flags: -O\n//@ edition: 2021\n//@ compile-flags: -C debuginfo=0\n")[0]
        self.assertEqual(result["compile_flags"][:4], ["--edition=2021", "-O", "-C", "debuginfo=0"])

    def test_edition_ranges_are_delegated_with_lower_bound_metadata(self):
        result = self.parse("//@ edition: 2018..2024\n")[0]
        self.assertEqual(result["edition"], "2018")
        self.assertEqual(result["delegated_to_compiletest"][0]["name"], "edition")
        for value in ("2024..2018", "..2024", "2021..2021", "2030"):
            self.assertTrue(self.parse(f"//@ edition: {value}\n")[0]["blocked"])

    def test_illegal_edition_and_incremental_flags_block(self):
        for value in ("--edition=2021", "--edition 2021", "-Cincremental=dir", "-C incremental=dir"):
            self.assertTrue(self.parse(f"//@ compile-flags: {value}\n")[0]["blocked"])

    def test_environment_splits_once_and_preserves_value_whitespace(self):
        result = self.parse("//@ exec-env: FOO= value=a=b  \n//@ exec-env: EMPTY\n//@ rustc-env: OTHER=z\n//@ unset-exec-env: OLD\n")[0]
        # FileDirectives trims the original line before extracting its value.
        self.assertEqual(result["exec_env"], {"FOO": " value=a=b", "EMPTY": ""})
        self.assertEqual(result["rustc_env"], {"OTHER": "z"})
        self.assertEqual(result["unset_exec_env"], ["OLD"])

    def test_target_and_host_conditions(self):
        result = self.parse("//@ run-pass\n//@ only-linux\n//@ only-x86_64\n//@ ignore-windows\n//@ ignore-cross-compile\n", host="x86_64-unknown-linux-gnu")[0]
        self.assertEqual(result["status"], "ready")
        result = self.parse("//@ run-pass\n//@ only-windows\n")[0]
        self.assertEqual(result["status"], "skipped")
        result = self.parse("//@ ignore-cross-compile\n", host="aarch64-unknown-linux-gnu")[0]
        self.assertEqual(result["status"], "skipped")

    def test_config_conditions_never_invent_facts(self):
        result = self.parse("//@ ignore-rustc-debug-assertions\n//@ only-nightly\n")[0]
        self.assertEqual([r["matched"] for r in result["requires"]], [None, None])
        self.assertEqual(len(result["delegated_to_compiletest"]), 2)

    def test_capabilities_true_false_and_unavailable(self):
        result = self.parse("//@ needs-threads\n//@ needs-unwind\n", capabilities={"needs-threads": True, "needs-unwind": False})[0]
        self.assertEqual(result["status"], "skipped")
        self.assertEqual([r["matched"] for r in result["requires"]], [True, False])

    def test_auxiliary_preserves_hash_and_protocol(self):
        auxiliary = self.path.parent / "auxiliary"
        auxiliary.mkdir()
        (auxiliary / "lib.rs").write_text("//@ edition: 2018\npub fn f() {}\n")
        result = self.parse("//@ run-pass\n//@ aux-build: lib.rs\n//@ aux-crate: renamed=lib.rs\n")[0]
        self.assertEqual(result["status"], "ready")
        self.assertEqual(result["auxiliary"][1]["name"], "renamed")
        self.assertEqual(result["auxiliary"][0]["directives"][0]["name"], "edition")
        self.assertEqual(len(result["auxiliary"][0]["source_sha256"]), 64)

    def test_auxiliary_missing_unknown_and_escape_block(self):
        self.assertTrue(self.parse("//@ aux-build: missing.rs\n")[0]["blocked"])
        self.assertTrue(self.parse("//@ aux-build: ../../../../escape.rs\n")[0]["blocked"])
        auxiliary = self.path.parent / "auxiliary"
        auxiliary.mkdir()
        (auxiliary / "lib.rs").write_text("//@ imaginary-directive\n")
        self.assertIn("auxiliary", " ".join(self.parse("//@ aux-build: lib.rs\n")[0]["blocked"]))

    def test_substitutions_are_expanded_or_explicitly_delegated(self):
        result = self.parse("//@ compile-flags: --cfg={{target}} -L{{build-base}}\n")[0]
        self.assertIn("--cfg=x86_64-unknown-linux-gnu", result["compile_flags"])
        self.assertEqual(result["delegated_to_compiletest"][0]["name"], "compile-flags")
        result = self.parse("//@ compile-flags: -L{{build-base}}\n", substitutions={"build-base": "/tmp/build"})[0]
        self.assertIn("-L/tmp/build", result["compile_flags"])
        self.assertTrue(self.parse("//@ compile-flags: {{made-up}}\n")[0]["blocked"])

    def test_output_sidecar_revision_selection_and_hash(self):
        self.path.with_suffix(".stderr").write_text("generic")
        self.path.with_suffix(".one.run.stdout").write_text("hello\n")
        results = self.parse("//@ revisions: one two\n//@ run-pass\n//@ check-run-results\n")
        self.assertEqual([x["kind"] for x in results[0]["expected_outputs"]], ["run.stdout"])
        self.assertEqual(results[1]["expected_outputs"], [])
        self.assertEqual(len(results[0]["expected_outputs"][0]["source_sha256"]), 64)

    def test_normalization_retains_rust_regex_and_newline(self):
        result = self.parse('//@ normalize-stderr: "[0-9]+" -> "number\\n"\n')[0]
        self.assertEqual(result["normalizations"], [{"kind": "normalize-stderr", "regex": "[0-9]+", "replacement": "number\n"}])
        self.assertTrue(self.parse('//@ normalize-stderr: invalid\n')[0]["blocked"])

    def test_run_fail_is_exit_range_not_assumed_101(self):
        result = self.parse("//@ run-fail\n//@ failure-status: 17\n", capabilities={"can-unwind": True})[0]
        self.assertEqual(result["run_expectation"], {"result": "fail", "exit_code_min": 1, "exit_code_max": 127, "allow_crash": False})
        self.assertIsNone(result["expected_failure_status"])
        self.assertEqual(result["failure_status"], 17)
        self.assertTrue(self.parse("//@ run-fail\n", capabilities={"can-unwind": False})[0]["run_expectation"]["allow_crash"])

    def test_multiple_modes_are_only_invalid_when_active_together(self):
        results = self.parse("//@ revisions: pass fail\n//@[pass] run-pass\n//@[fail] check-fail\n")
        self.assertTrue(all(not r["blocked"] for r in results))
        self.assertTrue(self.parse("//@ run-pass\n//@ run-fail\n")[0]["blocked"])

    def aux(self, name, text):
        path = self.path.parent / "auxiliary" / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text)
        return path

    def auxiliary_blocks(self, text):
        return protocol.auxiliary_profile_blocks(self.parse(text)[0], self.root)

    def test_aux_profile_default_dylib_overrides_source_rlib(self):
        self.aux("lib.rs", '#![crate_type = "rlib"]\npub fn f() {}\n')
        reasons = self.auxiliary_blocks("//@ run-pass\n//@ no-prefer-dynamic\n//@ aux-build: lib.rs\n")
        self.assertIn("own active no-prefer-dynamic is absent", " ".join(reasons))

    def test_aux_profile_own_flag_and_rlib_are_static(self):
        self.aux("lib.rs", '//@ no-prefer-dynamic\n#![crate_type = "rlib"]\npub fn f() {}\n')
        self.assertEqual(self.auxiliary_blocks("//@ aux-build: lib.rs\n"), [])
        self.aux("lib.rs", '//@ no-prefer-dynamic\n//@ compile-flags: --crate-type lib\n')
        self.assertEqual(self.auxiliary_blocks("//@ aux-build: lib.rs\n"), [])
        self.aux("lib.rs", '//@ no-prefer-dynamic\n//@ compile-flags: --crate-type=rlib\n')
        self.assertEqual(self.auxiliary_blocks("//@ aux-build: lib.rs\n"), [])

    def test_aux_profile_revision_properties_do_not_inherit(self):
        self.aux("lib.rs", '//@[static] no-prefer-dynamic\n#![crate_type = "rlib"]\n')
        variants = self.parse("//@ revisions: static dynamic\n//@ no-prefer-dynamic\n//@ aux-build: lib.rs\n")
        self.assertEqual(protocol.auxiliary_profile_blocks(variants[0], self.root), [])
        self.assertIn("appends --crate-type=dylib", " ".join(protocol.auxiliary_profile_blocks(variants[1], self.root)))

    def test_aux_profile_crate_type_unknown_dynamic_multiple_or_conditional_block(self):
        for source in (
            "", '#![crate_type = "dylib"]', '#![crate_type = "bin"]',
            '#![crate_type = "staticlib"]', '#![crate_type = "cdylib"]',
            '#![crate_type = "rlib"]\n#![crate_type = "lib"]',
            '#![cfg_attr(foo, crate_type = "rlib")]',
            '#![crate_type = concat!("r", "lib")]',
            '//@ compile-flags: --crate-type=lib,rlib',
            '//@ compile-flags: --crate-type={{target}}',
        ):
            with self.subTest(source=source):
                self.aux("lib.rs", "//@ no-prefer-dynamic\n" + source)
                self.assertTrue(self.auxiliary_blocks("//@ aux-build: lib.rs\n"))

    def test_aux_profile_ignores_fake_type_in_comments_and_strings(self):
        self.aux("lib.rs", '''//@ no-prefer-dynamic
/* outer /* nested */ #![crate_type = "dylib"] */
#![doc = "crate_type is documented"]
#![crate_type = "rlib"]
const TEXT: &str = r###"\n#![crate_type = "dylib"]\n"###;
''')
        self.assertEqual(self.auxiliary_blocks("//@ aux-build: lib.rs\n"), [])

    def test_aux_profile_aux_crate_retains_official_dylib_filename_contract(self):
        self.aux("lib.rs", '//@ no-prefer-dynamic\n#![crate_type = "rlib"]\n')
        reasons = self.auxiliary_blocks("//@ aux-crate: renamed=lib.rs\n")
        self.assertIn("--extern expects a dylib filename", " ".join(reasons))

    def test_aux_profile_bin_is_allowed_but_compiler_loaders_are_blocked(self):
        self.aux("bin.rs", 'fn main() {}\n')
        self.assertEqual(self.auxiliary_blocks("//@ aux-bin: bin.rs\n"), [])
        for kind in ("proc-macro", "aux-codegen-backend"):
            with self.subTest(kind=kind):
                self.assertIn("dynamic compiler loading", " ".join(self.auxiliary_blocks(f"//@ {kind}: bin.rs\n")))

    def test_aux_profile_transitive_paths_use_main_auxiliary_directory(self):
        self.aux("sub/first.rs", '//@ no-prefer-dynamic\n//@ aux-build: second.rs\n#![crate_type = "rlib"]\n')
        self.aux("second.rs", '//@ no-prefer-dynamic\n#![crate_type = "lib"]\n')
        self.assertEqual(self.auxiliary_blocks("//@ aux-build: sub/first.rs\n"), [])
        self.aux("second.rs", '#![crate_type = "rlib"]\n')
        self.assertIn("second.rs: own active", " ".join(self.auxiliary_blocks("//@ aux-build: sub/first.rs\n")))

    def test_aux_profile_transitive_revision_unknown_missing_and_cycles_block(self):
        self.aux("first.rs", '//@ no-prefer-dynamic\n//@ aux-build: second.rs\n#![crate_type = "rlib"]\n')
        self.aux("second.rs", '//@ no-prefer-dynamic\n//@ aux-build: first.rs\n#![crate_type = "rlib"]\n')
        self.assertIn("cyclic auxiliary dependency", " ".join(self.auxiliary_blocks("//@ aux-build: first.rs\n")))
        self.aux("second.rs", '//@ imaginary-protocol\n')
        self.assertIn("unknown upstream directive", " ".join(self.auxiliary_blocks("//@ aux-build: first.rs\n")))
        (self.path.parent / "auxiliary" / "second.rs").unlink()
        self.assertIn("source is missing", " ".join(self.auxiliary_blocks("//@ aux-build: first.rs\n")))

    def test_aux_profile_transitive_selection_uses_same_revision(self):
        self.aux("first.rs", '//@ no-prefer-dynamic\n//@[one] aux-build: missing.rs\n#![crate_type = "rlib"]\n')
        variants = self.parse("//@ revisions: one two\n//@ aux-build: first.rs\n")
        self.assertTrue(protocol.auxiliary_profile_blocks(variants[0], self.root))
        self.assertEqual(protocol.auxiliary_profile_blocks(variants[1], self.root), [])


if __name__ == "__main__":
    unittest.main()
