#![allow(unused, non_snake_case, unconditional_panic)]
// tests/function_calls.rs::register_arguments_are_loaded_before_the_call
pub mod case_000 {
fn id(x: i64) -> i64 { return x; } #[inline(never)] pub fn test_fn() -> i64 { return id(42); }
}
// tests/function_calls.rs::stack_arguments_are_stored_before_the_call
pub mod case_001 {
fn sum8(a: i64, b: i64, c: i64, d: i64, e: i64, f: i64, g: i64, h: i64) -> i64 { return a + b + c + d + e + f + g + h; } #[inline(never)] pub fn test_fn() -> i64 { return sum8(1, 2, 3, 4, 5, 6, 7, 8); }
}
// tests/loops.rs::calls_and_relocations_survive_loops
pub mod case_002 {
fn sum_to(limit: i64) -> i64 { let mut sum = 0; let mut i = 1; while i <= limit { sum = sum + i; i = i + 1; } return sum; } #[inline(never)] pub fn test_fn() -> i64 { return sum_to(10); }
}
// tests/forward_calls.rs::forward_calls_use_declaration_order_indices
pub mod case_003 {
#[inline(never)] pub fn test_fn() -> i64 { return later(40); } fn padding() -> i64 { return 0; } fn later(value: i64) -> i64 { return value + 2; }
}
// tests/forward_calls.rs::self_recursion_emits_a_call_to_the_current_function
pub mod case_004 {
fn factorial(value: i64) -> i64 { if value <= 1 { return 1; } return value * factorial(value - 1); } #[inline(never)] pub fn test_fn() -> i64 { return factorial(5); }
}
// tests/forward_calls.rs::mutual_recursion_uses_each_forward_signature
pub mod case_005 {
#[inline(never)] pub fn test_fn() -> i64 { return if even(10) { 42 } else { 0 }; } fn even(value: u64) -> bool { if value == 0 { return true; } return odd(value - 1); } fn odd(value: u64) -> bool { if value == 0 { return false; } return even(value - 1); }
}
// tests/forward_calls.rs::signature_collection_checks_types_and_skips_nested_bodies
pub mod case_006 {
fn first(value: i64) -> i64 { let mut index = 0; while index < 2 { if value > 0 { index += 1; } else { index += 2; } } return second(value); } fn second(value: i64) -> i64 { return value + 2; } #[inline(never)] pub fn test_fn() -> i64 { return first(40); }
}
// tests/const_functions.rs::const_fn_forward_calls_use_the_prescanned_signature
pub mod case_007 {
#[inline(never)] pub const fn test_fn() -> i64 { return later(40); } pub const fn later(value: i64) -> i64 { return value + 2; }
}
// tests/const_functions.rs::const_fn_recursion_emits_a_runtime_call
pub mod case_008 {
const fn countdown(value: i64) -> i64 { if value == 0 { return 42; } return countdown(value - 1); } #[inline(never)] pub const fn test_fn() -> i64 { return countdown(3); }
}
// tests/const_functions.rs::const_fn_supports_all_visibilities_and_preserves_object_symbols
pub mod case_009 {
#[inline(never)] pub const fn test_fn() -> i64 { return crate_visible(); } const fn private() -> i64 { return 0; } pub(crate) const fn crate_visible() -> i64 { return 42; } pub(self) const fn self_visible() -> i64 { return 0; } pub(super) const fn super_visible() -> i64 { return 0; }
}
// tests/early_return.rs::bool_early_returns_work_across_function_calls
pub mod case_010 {
fn decide(value: i64) -> bool { if value > 0 { return true; } return false; } #[inline(never)] pub fn test_fn() -> i64 { return if decide(42) { 42 } else { 0 }; }
}
// tests/bool_ops.rs::bool_calls_use_function_signatures
pub mod case_011 {
fn positive(value: i64) -> bool { return value > 0; } #[inline(never)] pub fn test_fn() -> i64 { return if positive(42) && !false { 42 } else { 0 }; }
}
// tests/bool_comparisons.rs::bool_comparisons_use_function_parameter_and_return_signatures
pub mod case_012 {
fn equal(a: bool, b: bool) -> bool { return a == b; } fn different(a: bool, b: bool) -> bool { return a != b; } #[inline(never)] pub fn test_fn() -> i64 { return if equal(true, false) != different(false, true) { 42 } else { 0 }; }
}
// tests/assignment_syntax.rs::mutable_parameters_survive_function_call_lowering
pub mod case_013 {
fn adjust(mut value: i64) -> i64 { value += 5; value *= 3; return value; } #[inline(never)] pub fn test_fn() -> i64 { return adjust(16); }
}
// tests/control_optimization.rs::value_numbering_never_crosses_function_boundaries
pub mod case_014 {
fn is_positive(value: i64) -> bool { return value > 0; } #[inline(never)] pub fn test_fn() -> i64 { let positive = is_positive(42); let valid = positive && !false; return if valid { 42 } else { 0 }; }
}
// tests/for_loops.rs::range_calls_and_invalid_syntax_are_checked
pub mod case_015 {
fn start() -> usize { return 1; } fn end() -> usize { return 5; } #[inline(never)] pub fn test_fn() -> usize { let mut sum = 0usize; for i in start()..end() { sum += i; } return sum; }
}
// tests/loop_and_tail.rs::tail_calls_work_after_a_range_loop
pub mod case_016 {
fn sum() -> i64 { let mut total = 0; for i in 0..5 { total += i; } total + 32 } #[inline(never)] pub fn test_fn() -> i64 { sum() }
}
// tests/expression_statements.rs::unit_calls_with_arguments_work_as_statements
pub mod case_017 {
fn helper(value: i64) { return; } #[inline(never)] pub fn test_fn() -> i64 { helper(1); return 42; }
}
// tests/expression_statements.rs::nonunit_calls_and_pure_expressions_can_be_discarded
pub mod case_018 {
fn answer() -> i64 { return 42; } #[inline(never)] pub fn test_fn() -> i64 { answer(); 1 + 2; 3 < 4; return answer(); }
}
// tests/expression_statements.rs::unit_values_can_be_bound_and_returned_from_unit_functions
pub mod case_019 {
fn helper() { return; } #[inline(never)] pub fn test_fn() { let value = helper(); value; return value; }
}
// tests/u64_ops.rs::u64_calls_and_mixed_type_errors_are_checked
pub mod case_020 {
fn halve(value: u64) -> u64 { return value >> 1; } #[inline(never)] pub fn test_fn() -> u64 { return halve(84u64); }
}
// tests/function_visibility.rs::supported_visibilities_do_not_change_function_names_or_indices
pub mod case_021 {
#[inline(never)] pub fn test_fn() -> i64 { return later(40); } fn plain() -> i64 { return 0; } pub(crate) fn later(value: i64) -> i64 { return value + 2; } pub(self) fn self_visible() -> i64 { return 0; } pub(super) fn super_visible() -> i64 { return 0; }
}
// tests/function_visibility.rs::visibility_preserves_forward_calls_and_native_symbols
pub mod case_022 {
#[inline(never)] pub fn test_fn() -> i64 { return later(40); } pub(self) fn padding() -> i64 { return 0; } pub(super) fn later(value: i64) -> i64 { return value + 2; }
}
// tests/redundant_moves.rs::calls_keep_only_required_argument_and_result_moves
pub mod case_023 {
fn id(value: i64) -> i64 { return value; } #[inline(never)] pub fn test_fn() -> i64 { return id(42); }
}
// tests/casts.rs::casts_work_in_calls_and_ranges
pub mod case_024 {
fn convert(value: i64) -> u64 { return value as u64; } #[inline(never)] pub fn test_fn() -> u64 { let mut total = 0usize; for i in 0..convert(4) { total += i as usize; } return total as u64; }
}
// tests/function_calls.rs::zero_argument_calls_emit_symbols_and_relocations
pub mod case_025 {
fn answer() -> i64 { return 42; }
#[inline(never)] pub fn test_fn() -> i64 { return answer(); }
}
