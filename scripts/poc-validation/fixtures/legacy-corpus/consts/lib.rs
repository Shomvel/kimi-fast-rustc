#![allow(unused, non_snake_case, unconditional_panic)]
// tests/const_expressions.rs::evaluates_arithmetic_bitwise_shift_parentheses_and_precedence
pub mod case_000 {
const VALUE: i64 = 40 + 2; #[inline(never)] pub fn test_fn() -> i64 { return VALUE; }
}
// tests/const_expressions.rs::evaluates_arithmetic_bitwise_shift_parentheses_and_precedence
pub mod case_001 {
const VALUE: i64 = 46 - 4; #[inline(never)] pub fn test_fn() -> i64 { return VALUE; }
}
// tests/const_expressions.rs::evaluates_arithmetic_bitwise_shift_parentheses_and_precedence
pub mod case_002 {
const VALUE: i64 = 6 * 7; #[inline(never)] pub fn test_fn() -> i64 { return VALUE; }
}
// tests/const_expressions.rs::evaluates_arithmetic_bitwise_shift_parentheses_and_precedence
pub mod case_003 {
const VALUE: i64 = 84 / 2; #[inline(never)] pub fn test_fn() -> i64 { return VALUE; }
}
// tests/const_expressions.rs::evaluates_arithmetic_bitwise_shift_parentheses_and_precedence
pub mod case_004 {
const VALUE: i64 = 127 % 85; #[inline(never)] pub fn test_fn() -> i64 { return VALUE; }
}
// tests/const_expressions.rs::evaluates_arithmetic_bitwise_shift_parentheses_and_precedence
pub mod case_005 {
const VALUE: i64 = 0x20 | 0x0a; #[inline(never)] pub fn test_fn() -> i64 { return VALUE; }
}
// tests/const_expressions.rs::evaluates_arithmetic_bitwise_shift_parentheses_and_precedence
pub mod case_006 {
const VALUE: i64 = 0x3f & 0x2a; #[inline(never)] pub fn test_fn() -> i64 { return VALUE; }
}
// tests/const_expressions.rs::evaluates_arithmetic_bitwise_shift_parentheses_and_precedence
pub mod case_007 {
const VALUE: i64 = 0x30 ^ 0x1a; #[inline(never)] pub fn test_fn() -> i64 { return VALUE; }
}
// tests/const_expressions.rs::evaluates_arithmetic_bitwise_shift_parentheses_and_precedence
pub mod case_008 {
const VALUE: i64 = 21i64 << 1; #[inline(never)] pub fn test_fn() -> i64 { return VALUE; }
}
// tests/const_expressions.rs::evaluates_arithmetic_bitwise_shift_parentheses_and_precedence
pub mod case_009 {
const VALUE: i64 = 168i64 >> 2; #[inline(never)] pub fn test_fn() -> i64 { return VALUE; }
}
// tests/const_expressions.rs::evaluates_arithmetic_bitwise_shift_parentheses_and_precedence
pub mod case_010 {
const VALUE: i64 = !(!42i64); #[inline(never)] pub fn test_fn() -> i64 { return VALUE; }
}
// tests/const_expressions.rs::evaluates_arithmetic_bitwise_shift_parentheses_and_precedence
pub mod case_011 {
const VALUE: i64 = (40 + 2); #[inline(never)] pub fn test_fn() -> i64 { return VALUE; }
}
// tests/const_expressions.rs::evaluates_arithmetic_bitwise_shift_parentheses_and_precedence
pub mod case_012 {
const VALUE: i64 = 1 + 2 * 3; #[inline(never)] pub fn test_fn() -> i64 { return VALUE; }
}
// tests/const_expressions.rs::evaluates_arithmetic_bitwise_shift_parentheses_and_precedence
pub mod case_013 {
const VALUE: i64 = (1 + 2) * 3; #[inline(never)] pub fn test_fn() -> i64 { return VALUE; }
}
// tests/const_expressions.rs::evaluates_arithmetic_bitwise_shift_parentheses_and_precedence
pub mod case_014 {
const VALUE: i64 = 0x30 | 0x0c ^ 0x02 & 0x03; #[inline(never)] pub fn test_fn() -> i64 { return VALUE; }
}
// tests/const_expressions.rs::evaluates_arithmetic_bitwise_shift_parentheses_and_precedence
pub mod case_015 {
const VALUE: i64 = -7 / 2; #[inline(never)] pub fn test_fn() -> i64 { return VALUE; }
}
// tests/const_expressions.rs::evaluates_arithmetic_bitwise_shift_parentheses_and_precedence
pub mod case_016 {
const VALUE: i64 = -7 % 2; #[inline(never)] pub fn test_fn() -> i64 { return VALUE; }
}
// tests/const_expressions.rs::evaluates_arithmetic_bitwise_shift_parentheses_and_precedence
pub mod case_017 {
const VALUE: i64 = -8i64 >> 2; #[inline(never)] pub fn test_fn() -> i64 { return VALUE; }
}
// tests/const_expressions.rs::evaluates_comparisons_boolean_not_and_short_circuiting
pub mod case_018 {
const VALUE: bool = 2 < 3; #[inline(never)] pub fn test_fn() -> bool { return VALUE; }
}
// tests/const_expressions.rs::evaluates_comparisons_boolean_not_and_short_circuiting
pub mod case_019 {
const VALUE: bool = 2 <= 2; #[inline(never)] pub fn test_fn() -> bool { return VALUE; }
}
// tests/const_expressions.rs::evaluates_comparisons_boolean_not_and_short_circuiting
pub mod case_020 {
const VALUE: bool = 3 > 2; #[inline(never)] pub fn test_fn() -> bool { return VALUE; }
}
// tests/const_expressions.rs::evaluates_comparisons_boolean_not_and_short_circuiting
pub mod case_021 {
const VALUE: bool = 3 >= 4; #[inline(never)] pub fn test_fn() -> bool { return VALUE; }
}
// tests/const_expressions.rs::evaluates_comparisons_boolean_not_and_short_circuiting
pub mod case_022 {
const VALUE: bool = 3 == 3; #[inline(never)] pub fn test_fn() -> bool { return VALUE; }
}
// tests/const_expressions.rs::evaluates_comparisons_boolean_not_and_short_circuiting
pub mod case_023 {
const VALUE: bool = 3 != 3; #[inline(never)] pub fn test_fn() -> bool { return VALUE; }
}
// tests/const_expressions.rs::evaluates_comparisons_boolean_not_and_short_circuiting
pub mod case_024 {
const VALUE: bool = true == !false; #[inline(never)] pub fn test_fn() -> bool { return VALUE; }
}
// tests/const_expressions.rs::evaluates_comparisons_boolean_not_and_short_circuiting
pub mod case_025 {
const VALUE: bool = false != false; #[inline(never)] pub fn test_fn() -> bool { return VALUE; }
}
// tests/const_expressions.rs::evaluates_comparisons_boolean_not_and_short_circuiting
pub mod case_026 {
const VALUE: bool = !false && (2 < 3) || false; #[inline(never)] pub fn test_fn() -> bool { return VALUE; }
}
// tests/const_expressions.rs::evaluates_comparisons_boolean_not_and_short_circuiting
pub mod case_027 {
const VALUE: bool = false && (1 / 0 == 0); #[inline(never)] pub fn test_fn() -> bool { return VALUE; }
}
// tests/const_expressions.rs::evaluates_comparisons_boolean_not_and_short_circuiting
pub mod case_028 {
const VALUE: bool = true || (1 % 0 == 0); #[inline(never)] pub fn test_fn() -> bool { return VALUE; }
}
// tests/const_expressions.rs::resolves_forward_and_transitive_dependencies_without_emitting_runtime_operators
pub mod case_029 {
const ANSWER: i64 = (BASE + OFFSET) * 2;const BASE: i64 = LIMIT - 1;const OFFSET: i64 = 1;const LIMIT: i64 = 21;const CHECKED: bool = ANSWER == 42 && !false || false;#[inline(never)] pub fn test_fn() -> i64 { return ANSWER; }
}
// tests/const_expressions.rs::preserves_signed_and_unsigned_boundaries
pub mod case_030 {
const VALUE: i64 = 9223372036854775807i64; #[inline(never)] pub fn test_fn() -> i64 { return VALUE; }
}
// tests/const_expressions.rs::preserves_signed_and_unsigned_boundaries
pub mod case_031 {
const VALUE: i64 = -9223372036854775808i64; #[inline(never)] pub fn test_fn() -> i64 { return VALUE; }
}
// tests/const_expressions.rs::preserves_signed_and_unsigned_boundaries
pub mod case_032 {
const VALUE: u64 = 18446744073709551615; #[inline(never)] pub fn test_fn() -> u64 { return VALUE; }
}
// tests/const_expressions.rs::preserves_signed_and_unsigned_boundaries
pub mod case_033 {
const VALUE: u64 = 0xffff_ffff_ffff_ffffu64 >> 63u64; #[inline(never)] pub fn test_fn() -> u64 { return VALUE; }
}
// tests/const_expressions.rs::preserves_signed_and_unsigned_boundaries
pub mod case_034 {
const VALUE: u64 = 1u64 << 63u64; #[inline(never)] pub fn test_fn() -> u64 { return VALUE; }
}
// tests/const_expressions.rs::preserves_signed_and_unsigned_boundaries
pub mod case_035 {
const VALUE: u64 = !0u64; #[inline(never)] pub fn test_fn() -> u64 { return VALUE; }
}
// tests/const_items.rs::constant_items_do_not_change_function_indices_and_const_fn_stays_a_function
pub mod case_036 {
const ANSWER: i64 = 42;const fn identity(value: i64) -> i64 { return value; }pub const UNUSED: bool = false;#[inline(never)] pub fn test_fn() -> i64 { return identity(ANSWER); }
}
// tests/integer_syntax.rs::radix_literals_preserve_signed_and_unsigned_values
pub mod case_037 {
#[inline(never)] pub fn test_fn() -> i64 { return 0x2a + 0x2A + 0o52 + 0o52 + 0b10_1010 + 0b101010; }
}
// tests/integer_syntax.rs::radix_literals_preserve_signed_and_unsigned_values
pub mod case_038 {
#[inline(never)] pub fn test_fn() -> u64 { let value: u64 = 0xffff_ffff_ffff_ffff; return value; }
}
// tests/const_items.rs::scalar_constants_support_visibilities_radices_suffixes_and_boolean_references
pub mod case_039 {
const SIGNED: i64 = -0x2a;const UNSIGNED: u64 = 0xffff_ffff_ffff_ffff;pub const WORD: isize = 0o52isize;pub(crate) const UWORD: usize = 0b10_1010usize;pub(self) const ENABLED: bool = true;pub(super) const DISABLED: bool = false;#[inline(never)] pub fn test_fn() -> i64 {if ENABLED && !DISABLED {return SIGNED + WORD as i64 + UWORD as i64 + 1 + UNSIGNED as i64;}return 0;}
}
// tests/const_items.rs::references_emit_typed_constants_and_locals_can_shadow_constant_names
pub mod case_040 {
const BASE: u64 = 0x28; const OFFSET: usize = 2usize;#[inline(never)] pub fn test_fn() -> u64 { return BASE + OFFSET as u64; }
}
// tests/const_items.rs::scalar_constants_support_visibilities_radices_suffixes_and_boolean_references
pub mod case_041 {
const MINIMUM: isize = -0x8000_0000_0000_0000isize;#[inline(never)] pub fn test_fn() -> isize { return MINIMUM; }
}
