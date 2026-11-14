#![allow(unused, non_snake_case, unconditional_panic)]
// tests/bool_comparisons.rs::bool_equality_and_inequality_cover_the_truth_table
pub mod case_000 {
#[inline(never)] pub fn test_fn(a: bool, b: bool) -> bool { return a == b; }
}
// tests/bool_comparisons.rs::bool_equality_and_inequality_cover_the_truth_table
pub mod case_001 {
#[inline(never)] pub fn test_fn(a: bool, b: bool) -> bool { return a != b; }
}
// tests/bool_ops.rs::bool_parameters_and_logical_operators_cover_truth_tables
pub mod case_002 {
#[inline(never)] pub fn test_fn(a: bool, b: bool) -> bool { return a && b; }
}
// tests/bool_ops.rs::bool_parameters_and_logical_operators_cover_truth_tables
pub mod case_003 {
#[inline(never)] pub fn test_fn(a: bool, b: bool) -> bool { return a || b; }
}
// tests/bool_comparisons.rs::bool_comparisons_preserve_short_circuit_evaluation_and_precedence
pub mod case_004 {
#[inline(never)] pub fn test_fn(a: bool, b: bool, divisor: i64) -> bool { return a == b && 1 / divisor == 0; }
}
// tests/bool_comparisons.rs::bool_comparisons_preserve_short_circuit_evaluation_and_precedence
pub mod case_005 {
#[inline(never)] pub fn test_fn(a: bool, b: bool, divisor: i64) -> bool { return a != b || 1 / divisor == 0; }
}
// tests/bool_comparisons.rs::bool_comparisons_accept_branch_phi_values
pub mod case_006 {
#[inline(never)] pub fn test_fn(a: bool, b: bool) -> bool { let expression_value = if a == b { a } else { b }; let mut statement_value = false; if a != b { statement_value = true; } else { statement_value = false; } return expression_value == statement_value; }
}
// tests/bool_comparisons.rs::integer_comparisons_keep_signed_and_unsigned_semantics
pub mod case_007 {
#[inline(never)] pub fn test_fn(a: i64, b: i64) -> bool { return a < b; }
}
// tests/bool_comparisons.rs::integer_comparisons_keep_signed_and_unsigned_semantics
pub mod case_008 {
#[inline(never)] pub fn test_fn(a: u64, b: u64) -> bool { return a < b; }
}
// tests/compare_immediates.rs::signed_left_constants_swap_to_the_inverse_right_immediate_comparison
pub mod case_009 {
#[inline(never)] pub fn test_fn(value: i64) -> bool { return 2 < value; }
}
// tests/compare_immediates.rs::unsigned_left_constants_swap_to_the_inverse_right_immediate_comparison
pub mod case_010 {
#[inline(never)] pub fn test_fn(value: u64) -> bool { return 2 < value; }
}
// tests/boolean_comparison_simplification.rs::signed_negated_comparisons_use_their_inverse
pub mod case_011 {
#[inline(never)] pub fn test_fn(a: i64, b: i64) -> bool { return !(a < b); }
}
// tests/boolean_comparison_simplification.rs::unsigned_negated_comparisons_use_their_inverse
pub mod case_012 {
#[inline(never)] pub fn test_fn(a: u64, b: u64) -> bool { return !(a < b); }
}
// tests/compare_immediates.rs::signed_left_constants_swap_to_the_inverse_right_immediate_comparison
pub mod case_013 {
#[inline(never)] pub fn test_fn(value: i64) -> bool { return 2 <= value; }
}
// tests/compare_immediates.rs::unsigned_left_constants_swap_to_the_inverse_right_immediate_comparison
pub mod case_014 {
#[inline(never)] pub fn test_fn(value: u64) -> bool { return 2 <= value; }
}
// tests/boolean_comparison_simplification.rs::signed_negated_comparisons_use_their_inverse
pub mod case_015 {
#[inline(never)] pub fn test_fn(a: i64, b: i64) -> bool { return !(a <= b); }
}
// tests/boolean_comparison_simplification.rs::unsigned_negated_comparisons_use_their_inverse
pub mod case_016 {
#[inline(never)] pub fn test_fn(a: u64, b: u64) -> bool { return !(a <= b); }
}
// tests/compare_immediates.rs::signed_left_constants_swap_to_the_inverse_right_immediate_comparison
pub mod case_017 {
#[inline(never)] pub fn test_fn(value: i64) -> bool { return 2 > value; }
}
// tests/compare_immediates.rs::unsigned_left_constants_swap_to_the_inverse_right_immediate_comparison
pub mod case_018 {
#[inline(never)] pub fn test_fn(value: u64) -> bool { return 2 > value; }
}
// tests/boolean_comparison_simplification.rs::signed_negated_comparisons_use_their_inverse
pub mod case_019 {
#[inline(never)] pub fn test_fn(a: i64, b: i64) -> bool { return !(a > b); }
}
// tests/boolean_comparison_simplification.rs::unsigned_negated_comparisons_use_their_inverse
pub mod case_020 {
#[inline(never)] pub fn test_fn(a: u64, b: u64) -> bool { return !(a > b); }
}
// tests/compare_immediates.rs::signed_left_constants_swap_to_the_inverse_right_immediate_comparison
pub mod case_021 {
#[inline(never)] pub fn test_fn(value: i64) -> bool { return 2 >= value; }
}
// tests/compare_immediates.rs::unsigned_left_constants_swap_to_the_inverse_right_immediate_comparison
pub mod case_022 {
#[inline(never)] pub fn test_fn(value: u64) -> bool { return 2 >= value; }
}
// tests/boolean_comparison_simplification.rs::signed_negated_comparisons_use_their_inverse
pub mod case_023 {
#[inline(never)] pub fn test_fn(a: i64, b: i64) -> bool { return !(a >= b); }
}
// tests/boolean_comparison_simplification.rs::unsigned_negated_comparisons_use_their_inverse
pub mod case_024 {
#[inline(never)] pub fn test_fn(a: u64, b: u64) -> bool { return !(a >= b); }
}
// tests/compare_immediates.rs::signed_left_constants_swap_to_the_inverse_right_immediate_comparison
pub mod case_025 {
#[inline(never)] pub fn test_fn(value: i64) -> bool { return 2 == value; }
}
// tests/boolean_comparison_simplification.rs::signed_negated_comparisons_use_their_inverse
pub mod case_026 {
#[inline(never)] pub fn test_fn(a: i64, b: i64) -> bool { return !(a == b); }
}
// tests/boolean_comparison_simplification.rs::unsigned_negated_comparisons_use_their_inverse
pub mod case_027 {
#[inline(never)] pub fn test_fn(a: u64, b: u64) -> bool { return !(a == b); }
}
// tests/compare_immediates.rs::signed_left_constants_swap_to_the_inverse_right_immediate_comparison
pub mod case_028 {
#[inline(never)] pub fn test_fn(value: i64) -> bool { return 2 != value; }
}
// tests/boolean_comparison_simplification.rs::signed_negated_comparisons_use_their_inverse
pub mod case_029 {
#[inline(never)] pub fn test_fn(a: i64, b: i64) -> bool { return !(a != b); }
}
// tests/boolean_comparison_simplification.rs::unsigned_negated_comparisons_use_their_inverse
pub mod case_030 {
#[inline(never)] pub fn test_fn(a: u64, b: u64) -> bool { return !(a != b); }
}
// tests/bool_ops.rs::comparisons_and_logical_not_return_bool
pub mod case_031 {
#[inline(never)] pub fn test_fn(value: i64) -> bool { let positive = value > 0; return !positive; }
}
// tests/bool_ops.rs::logical_precedence_supports_not_and_parentheses
pub mod case_032 {
#[inline(never)] pub fn test_fn(a: bool, b: bool, c: bool) -> bool { return a || b && !c; }
}
// tests/u64_ops.rs::unsigned_comparisons_do_not_use_signed_ordering
pub mod case_033 {
#[inline(never)] pub fn test_fn(a: u64, b: u64) -> bool { return a < b; }
}
// tests/u64_ops.rs::unsigned_comparisons_do_not_use_signed_ordering
pub mod case_034 {
#[inline(never)] pub fn test_fn(a: u64, b: u64) -> bool { return a <= b; }
}
// tests/u64_ops.rs::unsigned_comparisons_do_not_use_signed_ordering
pub mod case_035 {
#[inline(never)] pub fn test_fn(a: u64, b: u64) -> bool { return a > b; }
}
// tests/u64_ops.rs::unsigned_comparisons_do_not_use_signed_ordering
pub mod case_036 {
#[inline(never)] pub fn test_fn(a: u64, b: u64) -> bool { return a >= b; }
}
// tests/bool_ops.rs::short_circuit_rhs_does_not_execute_division
pub mod case_037 {
#[inline(never)] pub fn test_fn(flag: bool) -> bool { let zero = 0; return flag && 1 / zero == 0; }
}
// tests/bool_ops.rs::short_circuit_rhs_does_not_execute_division
pub mod case_038 {
#[inline(never)] pub fn test_fn(flag: bool) -> bool { let zero = 0; return flag || 1 / zero == 0; }
}
// tests/boolean_comparison_simplification.rs::comparison_with_true_or_false_removes_the_outer_comparison
pub mod case_039 {
#[inline(never)] pub fn test_fn(a: i64, b: i64) -> bool { return (a < b) == true; }
}
// tests/boolean_comparison_simplification.rs::comparison_with_true_or_false_removes_the_outer_comparison
pub mod case_040 {
#[inline(never)] pub fn test_fn(a: i64, b: i64) -> bool { return (a < b) != false; }
}
// tests/boolean_comparison_simplification.rs::comparison_with_true_or_false_removes_the_outer_comparison
pub mod case_041 {
#[inline(never)] pub fn test_fn(a: i64, b: i64) -> bool { return (a < b) == false; }
}
// tests/boolean_comparison_simplification.rs::comparison_with_true_or_false_removes_the_outer_comparison
pub mod case_042 {
#[inline(never)] pub fn test_fn(a: i64, b: i64) -> bool { return (a < b) != true; }
}
// tests/boolean_comparison_simplification.rs::negated_branch_comparison_does_not_materialize_two_booleans
pub mod case_043 {
#[inline(never)] pub fn test_fn(a: i64) -> i64 { return if !(a < 4) { 42 } else { 0 }; }
}
// tests/casts.rs::numeric_to_bool_and_numeric_conditions_are_rejected
pub mod case_044 {
#[inline(never)] pub fn test_fn(flag: bool) -> bool { return flag as bool; }
}
// tests/machine_differential.rs::every_supported_comparison_matches_at_boundaries
pub mod case_045 {
#[inline(never)] pub fn test_fn(left: i64, right: i64) -> bool { return left < right; }
}
// tests/machine_differential.rs::every_supported_comparison_matches_at_boundaries
pub mod case_046 {
#[inline(never)] pub fn test_fn(left: i64, right: i64) -> bool { return left <= right; }
}
// tests/machine_differential.rs::every_supported_comparison_matches_at_boundaries
pub mod case_047 {
#[inline(never)] pub fn test_fn(left: i64, right: i64) -> bool { return left > right; }
}
// tests/machine_differential.rs::every_supported_comparison_matches_at_boundaries
pub mod case_048 {
#[inline(never)] pub fn test_fn(left: i64, right: i64) -> bool { return left >= right; }
}
// tests/machine_differential.rs::every_supported_comparison_matches_at_boundaries
pub mod case_049 {
#[inline(never)] pub fn test_fn(left: i64, right: i64) -> bool { return left == right; }
}
// tests/machine_differential.rs::every_supported_comparison_matches_at_boundaries
pub mod case_050 {
#[inline(never)] pub fn test_fn(left: i64, right: i64) -> bool { return left != right; }
}
// tests/zero_compare.rs::native_compare_and_direct_branch_use_test_for_zero
pub mod case_051 {
#[inline(never)] pub fn test_fn(value: i64) -> bool { return value < 0; }
}
// tests/zero_compare.rs::native_compare_and_direct_branch_use_test_for_zero
pub mod case_052 {
#[inline(never)] pub fn test_fn(value: i64) -> i64 { return if value < 0 { 42 } else { 0 }; }
}
// tests/zero_compare.rs::native_compare_and_direct_branch_use_test_for_zero
pub mod case_053 {
#[inline(never)] pub fn test_fn(value: u64) -> bool { return value < 0; }
}
// tests/zero_compare.rs::native_compare_and_direct_branch_use_test_for_zero
pub mod case_054 {
#[inline(never)] pub fn test_fn(value: u64) -> i64 { return if value < 0 { 42 } else { 0 }; }
}
// tests/zero_compare.rs::native_compare_and_direct_branch_use_test_for_zero
pub mod case_055 {
#[inline(never)] pub fn test_fn(value: i64) -> bool { return value <= 0; }
}
// tests/zero_compare.rs::native_compare_and_direct_branch_use_test_for_zero
pub mod case_056 {
#[inline(never)] pub fn test_fn(value: i64) -> i64 { return if value <= 0 { 42 } else { 0 }; }
}
// tests/zero_compare.rs::native_compare_and_direct_branch_use_test_for_zero
pub mod case_057 {
#[inline(never)] pub fn test_fn(value: u64) -> bool { return value <= 0; }
}
// tests/zero_compare.rs::native_compare_and_direct_branch_use_test_for_zero
pub mod case_058 {
#[inline(never)] pub fn test_fn(value: u64) -> i64 { return if value <= 0 { 42 } else { 0 }; }
}
// tests/zero_compare.rs::native_compare_and_direct_branch_use_test_for_zero
pub mod case_059 {
#[inline(never)] pub fn test_fn(value: i64) -> bool { return value > 0; }
}
// tests/zero_compare.rs::native_compare_and_direct_branch_use_test_for_zero
pub mod case_060 {
#[inline(never)] pub fn test_fn(value: i64) -> i64 { return if value > 0 { 42 } else { 0 }; }
}
// tests/zero_compare.rs::native_compare_and_direct_branch_use_test_for_zero
pub mod case_061 {
#[inline(never)] pub fn test_fn(value: u64) -> bool { return value > 0; }
}
// tests/zero_compare.rs::native_compare_and_direct_branch_use_test_for_zero
pub mod case_062 {
#[inline(never)] pub fn test_fn(value: u64) -> i64 { return if value > 0 { 42 } else { 0 }; }
}
// tests/zero_compare.rs::native_compare_and_direct_branch_use_test_for_zero
pub mod case_063 {
#[inline(never)] pub fn test_fn(value: i64) -> bool { return value >= 0; }
}
// tests/zero_compare.rs::native_compare_and_direct_branch_use_test_for_zero
pub mod case_064 {
#[inline(never)] pub fn test_fn(value: i64) -> i64 { return if value >= 0 { 42 } else { 0 }; }
}
// tests/zero_compare.rs::native_compare_and_direct_branch_use_test_for_zero
pub mod case_065 {
#[inline(never)] pub fn test_fn(value: u64) -> bool { return value >= 0; }
}
// tests/zero_compare.rs::native_compare_and_direct_branch_use_test_for_zero
pub mod case_066 {
#[inline(never)] pub fn test_fn(value: u64) -> i64 { return if value >= 0 { 42 } else { 0 }; }
}
// tests/zero_compare.rs::native_compare_and_direct_branch_use_test_for_zero
pub mod case_067 {
#[inline(never)] pub fn test_fn(value: i64) -> bool { return value == 0; }
}
// tests/zero_compare.rs::native_compare_and_direct_branch_use_test_for_zero
pub mod case_068 {
#[inline(never)] pub fn test_fn(value: i64) -> i64 { return if value == 0 { 42 } else { 0 }; }
}
// tests/zero_compare.rs::native_compare_and_direct_branch_use_test_for_zero
pub mod case_069 {
#[inline(never)] pub fn test_fn(value: i64) -> bool { return value != 0; }
}
// tests/zero_compare.rs::native_compare_and_direct_branch_use_test_for_zero
pub mod case_070 {
#[inline(never)] pub fn test_fn(value: i64) -> i64 { return if value != 0 { 42 } else { 0 }; }
}
// tests/immediate_arithmetic.rs::bitwise_multiply_and_compare_support_immediates
pub mod case_071 {
#[inline(never)] pub fn test_fn(value: i64) -> bool { return value == 42; }
}
