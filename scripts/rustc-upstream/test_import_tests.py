"""Capability regressions observed while importing unchanged upstream tests."""

import importlib.util
from pathlib import Path
import unittest


SPEC = importlib.util.spec_from_file_location("import_tests", Path(__file__).with_name("import_tests.py"))
assert SPEC is not None and SPEC.loader is not None
IMPORTER = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(IMPORTER)


class FloatLiteralClassification(unittest.TestCase):
    def test_decimal_suffixes_and_exponents_require_float_support(self):
        for token in ["3f32", "6f64", "0f16", "42f128", "3_f32", "1_000_f64", "1e6f32", "1_2e+3_f64"]:
            with self.subTest(token=token):
                self.assertIn("capability:floating-point", IMPORTER.initial_blocks(f"let x = {token};", []))

    def test_radix_integer_digits_are_not_float_suffixes(self):
        for token in ["0xff32", "0x3f32", "0xff_f32", "0xdead_beef_f64", "0xF32", "0o7_32", "0b1_0010", "3u32", "6_i64"]:
            with self.subTest(token=token):
                self.assertNotIn("capability:floating-point", IMPORTER.initial_blocks(f"let x = {token};", []))

    def test_compound_assignment_float_cases_are_not_hidden(self):
        source = "let mut x = 3f32; x *= &3f32; assert_eq!(x, 9f32); let mut y = 6f64; y /= &2f64; assert_eq!(y, 3f64);"
        self.assertNotIn("capability:floating-point", IMPORTER.initial_blocks(source, [], classifier_version=1))
        self.assertEqual(IMPORTER.initial_blocks(source, []), ["capability:floating-point"])

    def test_original_type_and_decimal_detection_remains(self):
        for source in ["let x: f32 = 3;", "let x = 3.5;", "let x = 4.9e-10_f64;"]:
            with self.subTest(source=source):
                self.assertIn("capability:floating-point", IMPORTER.initial_blocks(source, []))


class WideIntegerClassification(unittest.TestCase):
    def test_all_radices_and_underscores_require_wide_integer_support(self):
        for token in ["1u128", "10i128", "1_u128", "842857_i128", "0xffu128", "0x8000_0000i128", "0o7_77_u128", "0b10_10_i128"]:
            with self.subTest(token=token):
                self.assertIn("capability:integer-wider-than-64-bits", IMPORTER.initial_blocks(f"let x = {token};", []))

    def test_suffix_lookalikes_do_not_block_normal_integer_programs(self):
        for token in ["0x128", "0xff128", "128u64", "128_i64", "value1u128", "0o128", "0b101_010"]:
            with self.subTest(token=token):
                self.assertNotIn("capability:integer-wider-than-64-bits", IMPORTER.initial_blocks(f"let x = {token};", []))

    def test_macro_type_and_literal_arguments_are_covered(self):
        for source in [
            "macro_rules! check { ($t:ty) => {fn check(x: $t) {}} } check!(u128);",
            "macro_rules! check { ($x:expr) => {let x = $x;} } check!(1u128);",
            "macro_rules! check { ($x:expr) => {let x = $x;} } check!(0x8000_0000_i128);",
        ]:
            with self.subTest(source=source):
                self.assertIn("capability:integer-wider-than-64-bits", IMPORTER.initial_blocks(source, []))

    def test_next_power_of_two_suffix_gap_is_explicit(self):
        source = "assert_eq!(((1u128 << 127) + 1).next_power_of_two(), 0);"
        self.assertNotIn("capability:integer-wider-than-64-bits", IMPORTER.initial_blocks(source, [], classifier_version=2))
        self.assertEqual(IMPORTER.initial_blocks(source, []), ["capability:integer-wider-than-64-bits"])


if __name__ == "__main__":
    unittest.main()
