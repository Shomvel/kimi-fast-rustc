#![allow(unused, non_snake_case, unconditional_panic)]
// tests/control_flow.rs::if_else_selects_the_expected_branch
pub mod case_000 {
#[inline(never)] pub fn test_fn(value: i64) -> i64 { return if value > 0 { 40 + 2 } else { 0 - 7 }; }
}
// tests/control_flow.rs::nested_if_preserves_control_and_phi_values
pub mod case_001 {
#[inline(never)] pub fn test_fn(value: i64) -> i64 { return if value > 10 { if value > 20 { 1 } else { 2 } } else { 3 }; }
}
// tests/statement_if.rs::if_else_merges_assignments_from_both_branches
pub mod case_002 {
#[inline(never)] pub fn test_fn(value: i64) -> i64 { let mut result = 0; if value > 0 { let adjustment = 40; result = value + adjustment; } else { let adjustment = 7; result = adjustment; } return result; }
}
// tests/statement_if.rs::if_without_else_preserves_the_unmodified_value
pub mod case_003 {
#[inline(never)] pub fn test_fn(value: i64) -> i64 { let mut result = 5; if value > 0 { result = result + value; } return result; }
}
// tests/statement_if.rs::nested_if_statements_merge_each_control_level
pub mod case_004 {
#[inline(never)] pub fn test_fn(value: i64) -> i64 { let mut result = 0; if value > 0 { if value > 10 { result = 2; } else { result = 1; } } else { result = -1; } return result; }
}
// tests/statement_if.rs::else_if_chains_merge_every_nested_branch
pub mod case_005 {
#[inline(never)] pub fn test_fn(value: i64) -> i64 { let mut result = 0; if value > 20 { result = 3; } else if value > 10 { result = 2; } else if value > 0 { result = 1; } else { result = -1; } return result; }
}
// tests/early_return.rs::early_if_return_skips_the_function_tail
pub mod case_006 {
#[inline(never)] pub fn test_fn(value: i64) -> i64 { if value > 10 { return 42; } return 7; }
}
// tests/early_return.rs::two_returning_arms_do_not_need_a_final_return
pub mod case_007 {
#[inline(never)] pub fn test_fn(value: i64) -> i64 { if value > 0 { return 42; } else { return 7; } }
}
// tests/shadowing.rs::let_rhs_uses_the_previous_binding_and_shadowing_can_change_type
pub mod case_008 {
#[inline(never)] pub fn test_fn(x: i64) -> i64 { let x = x + 1; let x = x * 2; let x = x == 6; if x { return 42; } return 0; }
}
// tests/shadowing.rs::branch_shadows_do_not_leak_into_outer_phis
pub mod case_009 {
#[inline(never)] pub fn test_fn(value: i64) -> i64 { let mut x = 1; if value > 0 { x = 5; let mut x = x + 1; x = 100; } else { x = 2; let x = 99; } return x; }
}
// tests/bool_optimizer_regression.rs::branch_local_values_do_not_replace_post_merge_expressions
pub mod case_010 {
#[inline(never)] pub fn test_fn(value: i64, flag: bool) -> i64 { let branch_value = if flag { value + 1 } else { 0 }; let after = value + 1; return branch_value + after; }
}
// tests/control_algebra.rs::additive_and_multiplicative_identities_are_removed
pub mod case_011 {
#[inline(never)] pub fn test_fn(value: i64, flag: bool) -> i64 { return if flag { value + 0 } else { value * 1 }; }
}
// tests/control_algebra.rs::zero_and_self_operations_become_constants
pub mod case_012 {
#[inline(never)] pub fn test_fn(value: i64, flag: bool) -> i64 { let zero = value & 0; return if flag { zero } else { value ^ value }; }
}
// tests/control_algebra.rs::self_comparison_simplifies_but_self_division_still_traps
pub mod case_013 {
#[inline(never)] pub fn test_fn(value: i64, flag: bool) -> i64 { let same = value == value; return if flag { same as i64 } else { value / value }; }
}
// tests/early_return.rs::unit_functions_support_early_and_implicit_returns
pub mod case_014 {
#[inline(never)] pub fn test_fn() { if true { return; } }
}
// tests/early_return.rs::unit_functions_support_early_and_implicit_returns
pub mod case_015 {
#[inline(never)] pub fn test_fn(flag: bool) { if flag { return; } else { return; } }
}
// tests/scalar_match.rs::signed_match_supports_negative_literals_blocks_and_trailing_commas
pub mod case_016 {
#[inline(never)] pub fn test_fn(value: i64) -> i64 { return match value { -1 => 40, 0 => { 40 + 1 }, 42 => 42, -9223372036854775808 => 43, _ => 0, }; }
}
// tests/scalar_match.rs::unsigned_and_word_alias_patterns_preserve_the_full_bit_range
pub mod case_017 {
#[inline(never)] pub fn test_fn(value: u64) -> u64 { let result = match value { 0 => 40u64, 18446744073709551615u64 => 42, _ => 1 }; return result; }
}
// tests/scalar_match.rs::unsigned_and_word_alias_patterns_preserve_the_full_bit_range
pub mod case_018 {
#[inline(never)] pub fn test_fn(value: isize) -> isize { return match value { -2isize => 42isize, _ => 0isize }; }
}
// tests/scalar_match.rs::unsigned_and_word_alias_patterns_preserve_the_full_bit_range
pub mod case_019 {
#[inline(never)] pub fn test_fn(value: usize) -> usize { return match value { 42usize => 42usize, _ => 0usize }; }
}
// tests/scalar_match.rs::bool_patterns_and_tail_match_select_the_expected_value
pub mod case_020 {
#[inline(never)] pub fn test_fn(flag: bool) -> i64 { match flag { true => 42, false => 0, _ => 1 } }
}
// tests/scalar_match.rs::tail_detection_stops_at_the_current_function_and_wildcard_only_matches_work
pub mod case_021 {
#[inline(never)] pub fn test_fn(value: i64) -> i64 { match value { _ => 42 } }
}
// tests/scalar_match.rs::nested_match_and_shadowed_bindings_keep_their_values
pub mod case_022 {
#[inline(never)] pub fn test_fn(value: i64) -> i64 { let value = value + 1; let result = 40; match value { 1 => result + 2, _ => match value { 3 => { result }, _ => 0, }, } }
}
// tests/scalar_match.rs::unsuffixed_arm_results_unify_with_unsigned_results
pub mod case_023 {
#[inline(never)] pub fn test_fn(value: u64) -> u64 { return match value { 0 => 40, 1 => 41, 2 => 42u64, _ => 0 }; }
}
// tests/match_ranges.rs::signed_ranges_include_zero_negative_values_and_full_type_boundaries
pub mod case_024 {
#[inline(never)] pub fn test_fn(value: i64) -> i64 { return match value { -9223372036854775808..=-1 => 40, 0..=0 => 42, 1..=9223372036854775807 => 41, _ => 0 }; }
}
// tests/match_ranges.rs::unsigned_ranges_use_unsigned_ordering_across_the_sign_bit
pub mod case_025 {
#[inline(never)] pub fn test_fn(value: u64) -> u64 { return match value { 0..=0 => 40u64, 1..=9223372036854775807 => 41, 9223372036854775808..=18446744073709551615 => 42, _ => 0 }; }
}
// tests/match_ranges.rs::unsigned_ranges_use_unsigned_ordering_across_the_sign_bit
pub mod case_026 {
#[inline(never)] pub fn test_fn(value: usize) -> usize { return match value { 0usize..=41usize => 0usize, 42usize..=18446744073709551615usize => 42usize, _ => 1usize }; }
}
// tests/match_ranges.rs::guards_fall_through_and_preserve_nested_matches_and_shadowing
pub mod case_027 {
#[inline(never)] pub fn test_fn(input: i64, limit: i64) -> i64 { let input = input + 1; let answer = 40; return match input { -2..=2 if input < limit => answer + 2, 3..=5 if false => 0, _ => match input { 3..=5 => { answer + 2 }, _ => 1, }, }; }
}
// tests/match_ranges.rs::guard_traps_and_short_circuiting_are_control_dependent_on_the_arm
pub mod case_028 {
#[inline(never)] pub fn test_fn(value: i64, divisor: i64) -> i64 { return match value { 0..=10 if value > 0 && 1 / divisor == 1 => 42, _ => 7 }; }
}
// tests/loop_and_tail.rs::tail_literals_variables_and_bools_do_not_need_return
pub mod case_029 {
#[inline(never)] pub fn test_fn() -> i64 { 42 }
}
// tests/loop_and_tail.rs::tail_literals_variables_and_bools_do_not_need_return
pub mod case_030 {
#[inline(never)] pub fn test_fn(value: i64) -> i64 { let result = value + 1; result }
}
// tests/loop_and_tail.rs::tail_literals_variables_and_bools_do_not_need_return
pub mod case_031 {
#[inline(never)] pub fn test_fn(value: i64) -> bool { value > 0 }
}
// tests/loop_and_tail.rs::tail_expressions_support_casts_and_arithmetic
pub mod case_032 {
#[inline(never)] pub fn test_fn(flag: bool) -> i64 { flag as i64 + 41 }
}
// tests/main_unit.rs::standard_main_forms_return_zero
pub mod case_033 {
#[inline(never)] pub fn test_fn() {}
}
// tests/main_unit.rs::standard_main_forms_return_zero
pub mod case_034 {
#[inline(never)] pub fn test_fn() { return; }
}
// tests/main_unit.rs::standard_main_forms_return_zero
pub mod case_035 {
#[inline(never)] pub fn test_fn() { let value = 1; }
}
// tests/branch_flags.rs::if_condition_uses_flags_without_a_temporary_bool
pub mod case_036 {
#[inline(never)] pub fn test_fn(value: i64) -> i64 { return if value > 0 { 42 } else { 0 }; }
}
// tests/branch_flags.rs::loop_condition_uses_flags_without_a_temporary_bool
pub mod case_037 {
#[inline(never)] pub fn test_fn() -> i64 { let mut sum = 0; let mut i = 0; while i < 5 { sum += i; i += 1; } return sum + 32; }
}
// tests/branch_flags.rs::actual_bool_results_keep_setcc_and_zero_extension
pub mod case_038 {
#[inline(never)] pub fn test_fn(value: i64) -> bool { return value == 0; }
}
// tests/branch_flags.rs::shared_comparison_still_materializes_its_bool_value
pub mod case_039 {
#[inline(never)] pub fn test_fn(value: i64) -> i64 { let positive = value > 0; return if positive { positive as i64 } else { 0 }; }
}
// tests/commutative_immediates.rs::left_constant_addition_uses_rhs_immediate
pub mod case_040 {
#[inline(never)] pub fn test_fn(value: i64) -> i64 { return 5 + value; }
}
// tests/commutative_immediates.rs::left_constant_multiplication_uses_rhs_immediate
pub mod case_041 {
#[inline(never)] pub fn test_fn(value: i64) -> i64 { return 3 * value; }
}
// tests/commutative_immediates.rs::left_constant_bitwise_and_uses_rhs_immediate
pub mod case_042 {
#[inline(never)] pub fn test_fn(value: i64) -> i64 { return 7 & value; }
}
// tests/commutative_immediates.rs::subtraction_keeps_the_constant_on_the_left
pub mod case_043 {
#[inline(never)] pub fn test_fn(value: i64) -> i64 { return 42 - value; }
}
// tests/compare_immediates.rs::left_constants_do_not_materialize_a_register
pub mod case_044 {
#[inline(never)] pub fn test_fn(value: i64) -> bool { return 2 < value; }
}
// tests/compare_immediates.rs::two_constant_comparisons_still_fold
pub mod case_045 {
#[inline(never)] pub fn test_fn() -> bool { return 2 < 3; }
}
// tests/control_algebra.rs::unsigned_division_and_remainder_by_one_are_simplified
pub mod case_046 {
#[inline(never)] pub fn test_fn(value: u64, flag: bool) -> u64 { return if flag { value / 1 } else { value % 1 }; }
}
// tests/control_optimization.rs::if_branch_constants_fold_in_place
pub mod case_047 {
#[inline(never)] pub fn test_fn(value: i64) -> i64 { return if value > 0 { 40 + 2 } else { 0 - 7 }; }
}
// tests/control_optimization.rs::pure_expressions_are_value_numbered_across_regions
pub mod case_048 {
#[inline(never)] pub fn test_fn(value: i64, flag: bool) -> i64 { let next = value + 1; return if flag { next } else { value + 1 }; }
}
// tests/control_optimization.rs::trapping_division_is_not_folded_or_removed
pub mod case_049 {
#[inline(never)] pub fn test_fn(flag: bool) -> i64 { let zero = 0; return if flag { 1 / zero } else { 2 }; }
}
// tests/immediate_shifts.rs::immediate_shifts_use_count_encodings_without_cl
pub mod case_050 {
#[inline(never)] pub fn test_fn(value: i64) -> i64 { return value << 3; }
}
// tests/native_copy_elision.rs::variable_shift_count_does_not_make_an_r11_rcx_round_trip
pub mod case_051 {
#[inline(never)] pub fn test_fn(value: i64, count: i64) -> i64 { return value << count; }
}
// tests/phi_moves.rs::acyclic_loop_phi_edges_do_not_allocate_temporaries
pub mod case_052 {
#[inline(never)] pub fn test_fn() -> i64 { let mut sum = 0; let mut i = 0; while i < 5 { sum += i; i += 1; } return sum + 32; }
}
// tests/redundant_moves.rs::if_phi_moves_do_not_emit_self_assignments
pub mod case_053 {
#[inline(never)] pub fn test_fn(value: i64) -> i64 { return if value > 0 { value + 1 } else { value }; }
}
// tests/redundant_moves.rs::loop_back_edge_moves_do_not_emit_self_assignments
pub mod case_054 {
#[inline(never)] pub fn test_fn() -> i64 { let mut sum = 0; for i in 0..5 { sum += i; } return sum + 32; }
}
// tests/shadowing.rs::each_shadow_has_its_own_mutability
pub mod case_055 {
#[inline(never)] pub fn test_fn(x: i64) -> i64 { let mut x = x + 40; x += 1; return x; }
}
// tests/zero_compare.rs::native_zero_compare_example_evaluates_to_42
pub mod case_056 {
#[inline(never)] pub fn test_fn(value: i64) -> i64 { let nonnegative = value >= 0; return if value == 0 { 40 + nonnegative as i64 + 1 } else { 0 }; }
}
// tests/zero_idiom.rs::native_zero_return_uses_xor_and_saves_four_bytes
pub mod case_057 {
#[inline(never)] pub fn test_fn() -> i64 { return 0; }
}
// tests/zero_idiom.rs::native_branch_example_uses_zero_idioms_and_returns_42
pub mod case_058 {
#[inline(never)] pub fn test_fn(value: i64) -> i64 { let zero = 0; return if value == 0 { 42 } else { zero }; }
}
// tests/zero_idiom.rs::native_loop_preserves_conditions_and_returns_42
pub mod case_059 {
#[inline(never)] pub fn test_fn() -> i64 { let mut total = 0; let mut i = 0; while i < 6 { total += 7; i += 1; } return total; }
}
// tests/shadowing.rs::branch_shadows_do_not_leak_into_outer_phis
pub mod case_060 {
#[inline(never)] pub fn test_fn(value: i64) -> i64 { let x = 40; if value > 0 { let x = true; } return x + 2; }
}
// tests/match_ranges.rs::guard_calls_execute_only_after_their_pattern_matches
pub mod case_061 {
fn effect() -> bool { return true; } #[inline(never)] pub fn test_fn(value: i64) -> i64 { return match value { 1..=2 if effect() => 42, _ => 0 }; }
}
// tests/match_ranges.rs::range_lowering_uses_unsigned_compares_control_regions_and_result_phi
pub mod case_062 {
#[inline(never)] pub fn test_fn(value: u64, flag: bool) -> u64 { return match value { 0..=42 if flag => 42u64, _ => 0 }; }
}
// tests/scalar_match.rs::match_lowers_to_if_chains_and_a_multiway_phi
pub mod case_063 {
#[inline(never)] pub fn test_fn(value: i64) -> i64 { return match value { 0 => 10, 1 => 20, _ => 30 }; }
}
// tests/scalar_match.rs::tail_detection_stops_at_the_current_function_and_wildcard_only_matches_work
pub mod case_064 {
#[inline(never)] pub fn test_fn(value: i64) -> i64 { match value { 1 => 42, _ => 0 } } fn later(value: i64) -> i64 { return value; }
}
// tests/control_flow.rs::direct_objects_resolve_control_relocations
pub mod case_065 {
#[inline(never)] pub fn test_fn(value: i64) -> i64 { return if value != 0 { 1 } else { 0 }; }
}
