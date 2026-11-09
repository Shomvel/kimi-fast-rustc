#![allow(unused, non_snake_case, unconditional_panic)]
// tests/loops.rs::loop_phi_copies_preserve_sequential_assignments
pub mod case_000 {
#[inline(never)] pub fn test_fn() -> i64 { let mut x = 1; let mut y = 2; let mut i = 0; while i < 1 { let next_x = y; let next_y = x; x = next_x; y = next_y; i = i + 1; } return x * 10 + y; }
}
// tests/loop_control.rs::break_preserves_values_at_the_exit_edge
pub mod case_001 {
#[inline(never)] pub fn test_fn() -> i64 { let mut value = 0; while true { value += 1; if value > 20 { break; } value += 10; } return value; }
}
// tests/loop_control.rs::continue_skips_only_the_rest_of_the_current_iteration
pub mod case_002 {
#[inline(never)] pub fn test_fn() -> i64 { let mut sum = 0; let mut i = 0; while i < 10 { i += 1; if i % 2 == 1 { continue; } sum += i; } return sum; }
}
// tests/loop_control.rs::one_terminal_if_arm_still_merges_the_reachable_arm
pub mod case_003 {
#[inline(never)] pub fn test_fn() -> i64 { let mut sum = 0; let mut i = 0; while i < 6 { i += 1; if i % 2 == 0 { continue; } sum += i; } return sum; }
}
// tests/loop_control.rs::continue_and_break_can_terminate_different_if_arms
pub mod case_004 {
#[inline(never)] pub fn test_fn() -> i64 { let mut i = 0; while true { i += 1; if i < 3 { continue; } else { break; } } return i; }
}
// tests/loop_control.rs::nested_loop_control_targets_the_innermost_loop
pub mod case_005 {
#[inline(never)] pub fn test_fn() -> i64 { let mut total = 0; let mut outer = 0; while outer < 3 { let mut inner = 0; while true { inner += 1; if inner == 2 { break; } total += 1; } outer += 1; } return total; }
}
// tests/loop_control.rs::break_exit_phi_preserves_bool_and_integer_types
pub mod case_006 {
#[inline(never)] pub fn test_fn() -> i64 { let mut i = 0; let mut done = false; while !done { i += 1; if i == 3 { done = true; break; } } return if done && i == 3 { 42 } else { 0 }; }
}
// tests/loop_values.rs::loop_expression_produces_the_break_value
pub mod case_007 {
#[inline(never)] pub fn test_fn() -> i64 { let answer = loop { break 40 + 2; }; return answer; }
}
// tests/loop_values.rs::loop_expression_can_be_the_function_tail
pub mod case_008 {
#[inline(never)] pub fn test_fn() -> i64 { loop { break 42; } }
}
// tests/loop_values.rs::conditional_break_and_continue_preserve_exit_snapshots
pub mod case_009 {
#[inline(never)] pub fn test_fn() -> i64 { let mut n = 0; let result = loop { n += 1; if n < 6 { continue; } break n; }; return result + n; }
}
// tests/loop_values.rs::nested_loop_expressions_target_the_innermost_loop
pub mod case_010 {
#[inline(never)] pub fn test_fn() -> i64 { let mut outer = 0; let answer = loop { outer += 1; let inner = loop { break outer; }; if outer < 6 { continue; } break inner * 7; }; return answer; }
}
// tests/loop_values.rs::loop_result_respects_shadowing_and_restores_the_outer_binding
pub mod case_011 {
#[inline(never)] pub fn test_fn() -> i64 { let value = 40; let answer = loop { let value = 41; if value == 41 { break value + 1; } break 0; }; return answer + value - 40; }
}
// tests/early_return.rs::return_inside_loop_leaves_without_updating_loop_phis
pub mod case_012 {
#[inline(never)] pub fn test_fn() -> i64 { let mut i = 0; while true { i += 1; if i == 4 { return i * 10 + 2; } } return 0; }
}
// tests/statement_if.rs::if_inside_loop_accumulates_selected_values
pub mod case_013 {
#[inline(never)] pub fn test_fn() -> i64 { let mut sum = 0; let mut i = 0; while i < 10 { if i % 2 == 0 { sum = sum + i; } i = i + 1; } return sum; }
}
// tests/statement_if.rs::branch_with_inner_loop_jumps_to_the_outer_merge
pub mod case_014 {
#[inline(never)] pub fn test_fn() -> i64 { let mut sum = 0; let mut i = 0; while i < 5 { if i < 3 { let mut j = 0; while j < 2 { sum = sum + 1; j = j + 1; } } else { sum = sum + 10; } i = i + 1; } return sum; }
}
// tests/for_loops.rs::exclusive_ranges_accumulate_and_empty_ranges_do_not_run
pub mod case_015 {
#[inline(never)] pub fn test_fn() -> i64 { let mut sum = 0; for i in 0..5 { sum += i; } for i in 5..0 { sum += 100; } for i in 2..5 { sum += i; } return sum; }
}
// tests/for_loops.rs::continue_advances_the_range_index
pub mod case_016 {
#[inline(never)] pub fn test_fn() -> i64 { let mut sum = 0; for i in 0..5 { if i % 2 == 1 { continue; } sum += i; } return sum; }
}
// tests/for_loops.rs::break_exits_the_range_loop
pub mod case_017 {
#[inline(never)] pub fn test_fn() -> i64 { let mut sum = 0; for i in 0..10 { if i == 4 { break; } sum += i; } return sum; }
}
// tests/for_loops.rs::nested_ranges_target_the_inner_loop_control_context
pub mod case_018 {
#[inline(never)] pub fn test_fn() -> i64 { let mut total = 0; for outer in 0..4 { for inner in 0..4 { if inner == 2 { continue; } total += 1; } } return total; }
}
// tests/for_loops.rs::early_return_works_inside_a_range_loop
pub mod case_019 {
#[inline(never)] pub fn test_fn() -> i64 { for i in 0..10 { if i == 3 { return i + 39; } } return 0; }
}
// tests/inclusive_ranges.rs::inclusive_ranges_include_the_end_and_skip_empty_ranges
pub mod case_020 {
#[inline(never)] pub fn test_fn() -> i64 { let mut sum = 0; for i in 0..=3 { sum += i; } for i in 3..=0 { sum += 100; } return sum; }
}
// tests/inclusive_ranges.rs::continue_on_the_last_inclusive_iteration_exits_cleanly
pub mod case_021 {
#[inline(never)] pub fn test_fn() -> i64 { let mut sum = 0; for i in 0..=5 { if i % 2 == 1 { continue; } sum += i; } return sum; }
}
// tests/inclusive_ranges.rs::inclusive_ranges_support_early_return
pub mod case_022 {
#[inline(never)] pub fn test_fn() -> i64 { for i in 0..=10 { if i == 3 { return i + 39; } } return 0; }
}
// tests/bool_comparisons.rs::bool_comparisons_accept_loop_phi_values
pub mod case_023 {
#[inline(never)] pub fn test_fn() -> i64 { let mut value = false; let mut changes = 0; let mut index = 0; while index < 4 { let next = !value; if next != value { changes += 1; } value = next; index += 1; } return if value == false && changes == 4 { 42 } else { 0 }; }
}
// tests/bool_ops.rs::bool_values_survive_loops_and_if_expressions
pub mod case_024 {
#[inline(never)] pub fn test_fn() -> i64 { let mut count = 0; let mut done = false; while !done && count < 4 { count += 1; if count == 4 { done = true; } } return if done && count == 4 { 42 } else { 0 }; }
}
// tests/assignment_syntax.rs::compound_assignments_work_inside_loops
pub mod case_025 {
#[inline(never)] pub fn test_fn() -> i64 { let mut sum = 0; let mut i = 0; while i < 10 { i += 1; sum += i; } sum *= 2; return sum; }
}
// tests/shadowing.rs::loop_shadows_do_not_leak_into_back_edges_or_breaks
pub mod case_026 {
#[inline(never)] pub fn test_fn() -> i64 { let mut x = 0; let mut i = 0; while i < 4 { x += 1; let mut x = x + 40; x += 100; i += 1; } return x; }
}
// tests/shadowing.rs::for_variables_shadow_outer_bindings_without_losing_outer_mutability
pub mod case_027 {
#[inline(never)] pub fn test_fn() -> i64 { let mut x = 40; for x in 0..3 { let x = x + 1; } x += 2; return x; }
}
// tests/control_optimization.rs::loop_conditions_fold_without_changing_control_nodes
pub mod case_028 {
#[inline(never)] pub fn test_fn() -> i64 { let mut i = 0; while 1 + 1 == 2 { i += 1; if i == 3 { break; } } return i + 39; }
}
// tests/loops.rs::mutable_accumulator_runs_zero_or_many_iterations
pub mod case_029 {
#[inline(never)] pub fn test_fn(limit: i64) -> i64 { let mut sum = 0; let mut i = 0; while i < limit { sum = sum + i; i = i + 1; } return sum; }
}
// tests/loops.rs::nested_loops_and_body_bindings_preserve_values
pub mod case_030 {
#[inline(never)] pub fn test_fn(limit: i64) -> i64 { let mut total = 0; let mut i = 0; while i < limit { let mut j = 0; while j < i { total = total + j; j = j + 1; } i = i + 1; } return total; }
}
// tests/loops.rs::loop_survives_register_pressure
pub mod case_031 {
#[inline(never)] pub fn test_fn(a: i64, b: i64, c: i64, d: i64, e: i64, f: i64, g: i64, h: i64, i: i64, j: i64, k: i64, l: i64) -> i64 { let mut n = 0; let mut total = 0; while n < 10 { total = total + n; n = n + 1; } return total + a + b + c + d + e + f + g + h + i + j + k + l; }
}
// tests/for_loops.rs::usize_is_a_u64_alias_in_ranges_and_bindings
pub mod case_032 {
#[inline(never)] pub fn test_fn() -> usize { let mut total = 0usize; for i in 0..5usize { total += i; } return total; }
}
// tests/inclusive_ranges.rs::inclusive_u64_max_does_not_wrap_to_zero
pub mod case_033 {
#[inline(never)] pub fn test_fn() -> u64 { let mut total = 0u64; for i in 18446744073709551615u64..=18446744073709551615u64 { total += i; } return total; }
}
// tests/inclusive_ranges.rs::inclusive_u64_max_does_not_wrap_to_zero
pub mod case_034 {
#[inline(never)] pub fn test_fn() -> u64 { let mut count = 0u64; for i in 18446744073709551614u64..=18446744073709551615u64 { count += 1; } return count; }
}
// tests/loop_values.rs::break_literals_unify_with_other_breaks_and_the_expected_type
pub mod case_035 {
#[inline(never)] pub fn test_fn(flag: bool) -> u64 { let result: u64 = loop { if flag { break 40; } break 2u64; }; return result; }
}
// tests/loop_values.rs::nested_immediate_loops_preserve_result_types
pub mod case_036 {
#[inline(never)] pub fn test_fn() -> u64 { let mut value: u64 = 40; let result = loop { let inner = loop { break value + 2; }; break inner; }; return result; }
}
// tests/loop_values.rs::unit_loop_expressions_allow_break_without_a_value
pub mod case_037 {
#[inline(never)] pub fn test_fn() { let result = loop { break; }; return result; }
}
// tests/loop_invariant.rs::pure_expressions_hoist_and_match_baseline_for_zero_or_many_iterations
pub mod case_038 {
#[inline(never)] pub fn test_fn(left: i64, right: i64, limit: i64) -> i64 { let mut sum = 0; let mut i = 0; while i < limit { sum += left * right + !right; if left < right { sum += 1; } i += 1; } return sum; }
}
// tests/loop_invariant.rs::nested_loops_hoist_only_past_loops_whose_phis_are_not_inputs
pub mod case_039 {
#[inline(never)] pub fn test_fn(left: i64, right: i64, outer_limit: i64, inner_limit: i64) -> i64 { let mut sum = 0; let mut i = 0; while i < outer_limit { let mut j = 0; while j < inner_limit { sum += left * right; sum += i * right; j += 1; } i += 1; } return sum; }
}
// tests/loop_invariant.rs::trapping_division_is_not_hoisted_or_removed
pub mod case_040 {
#[inline(never)] pub fn test_fn(left: i64, divisor: i64, limit: i64) -> i64 { let mut sum = 0; let mut i = 0; while i < limit { sum += left / divisor; i += 1; } return sum; }
}
// tests/loop_and_tail.rs::unconditional_loop_supports_break_and_continue
pub mod case_041 {
#[inline(never)] pub fn test_fn() -> i64 { let mut sum = 0; let mut i = 0; loop { i += 1; if i > 10 { break; } if i % 2 == 1 { continue; } sum += i; } return sum + 12; }
}
// tests/bool_optimizer_regression.rs::loop_exit_expressions_use_current_phi_values
pub mod case_042 {
#[inline(never)] pub fn test_fn() -> i64 { let mut value = 1; let mut index = 0; while index < 3 { let inside = value + 5; if inside == 0 { value = 7; } value += 1; index += 1; } return value + 5; }
}
// tests/bool_optimizer_regression.rs::loop_exit_expressions_use_current_phi_values
pub mod case_043 {
#[inline(never)] pub fn test_fn() -> i64 { let mut value = 1; let mut index = 0; while index < 3 { let inside = !value; if inside == 0 { value = 7; } value += 1; index += 1; } return !value; }
}
// tests/bool_optimizer_regression.rs::loop_exit_expressions_use_current_phi_values
pub mod case_044 {
#[inline(never)] pub fn test_fn() -> i64 { let mut value = 1; let mut index = 0; while index < 3 { let inside = value == 4; if inside { value = 7; } value += 1; index += 1; } return if value == 4 { 42 } else { 0 }; }
}
// tests/shadowing.rs::loop_shadows_do_not_leak_into_back_edges_or_breaks
pub mod case_045 {
#[inline(never)] pub fn test_fn() -> i64 { let mut x = 0; let mut i = 0; while i < 10 { x += 1; let x = 100; if i == 2 { break; } i += 1; } return x; }
}
// tests/shadowing.rs::for_variables_shadow_outer_bindings_without_losing_outer_mutability
pub mod case_046 {
#[inline(never)] pub fn test_fn() -> i64 { let x = 3; let mut sum = 0; for x in 0..x { sum += x; } return sum; }
}
// tests/loop_values.rs::break_literals_unify_with_other_breaks_and_the_expected_type
pub mod case_047 {
#[inline(never)] pub fn test_fn() -> u64 { loop { break 42; } }
}
// tests/loop_values.rs::unit_loop_expressions_allow_break_without_a_value
pub mod case_048 {
#[inline(never)] pub fn test_fn() { loop { break; } }
}
