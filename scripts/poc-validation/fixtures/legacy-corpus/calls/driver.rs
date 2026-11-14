extern crate legacy_calls;
fn main() {
let selected = std::env::args().nth(1);
match selected.as_deref() {
None => {
println!("function_calls__register_arguments_are_loaded_before_the_call:0={}", (legacy_calls::case_000::test_fn()) as i64);
println!("function_calls__stack_arguments_are_stored_before_the_call:0={}", (legacy_calls::case_001::test_fn()) as i64);
println!("loops__calls_and_relocations_survive_loops:0={}", (legacy_calls::case_002::test_fn()) as i64);
println!("forward_calls__forward_calls_use_declaration_order_indices:0={}", (legacy_calls::case_003::test_fn()) as i64);
println!("forward_calls__self_recursion_emits_a_call_to_the_current_function:0={}", (legacy_calls::case_004::test_fn()) as i64);
println!("forward_calls__mutual_recursion_uses_each_forward_signature:0={}", (legacy_calls::case_005::test_fn()) as i64);
println!("forward_calls__signature_collection_checks_types_and_skips_nested_bodies:0={}", (legacy_calls::case_006::test_fn()) as i64);
println!("const_functions__const_fn_forward_calls_use_the_prescanned_signature:0={}", (legacy_calls::case_007::test_fn()) as i64);
println!("const_functions__const_fn_recursion_emits_a_runtime_call:0={}", (legacy_calls::case_008::test_fn()) as i64);
println!("const_functions__const_fn_supports_all_visibilities_and_preserves_object_symbols:0={}", (legacy_calls::case_009::test_fn()) as i64);
println!("early_return__bool_early_returns_work_across_function_calls:0={}", (legacy_calls::case_010::test_fn()) as i64);
println!("bool_ops__bool_calls_use_function_signatures:0={}", (legacy_calls::case_011::test_fn()) as i64);
println!("bool_comparisons__bool_comparisons_use_function_parameter_and_return_signatures:0={}", (legacy_calls::case_012::test_fn()) as i64);
println!("assignment_syntax__mutable_parameters_survive_function_call_lowering:0={}", (legacy_calls::case_013::test_fn()) as i64);
println!("control_optimization__value_numbering_never_crosses_function_boundaries:0={}", (legacy_calls::case_014::test_fn()) as i64);
println!("for_loops__range_calls_and_invalid_syntax_are_checked:0={}", (legacy_calls::case_015::test_fn()) as i64);
println!("loop_and_tail__tail_calls_work_after_a_range_loop:0={}", (legacy_calls::case_016::test_fn()) as i64);
println!("expression_statements__unit_calls_with_arguments_work_as_statements:0={}", (legacy_calls::case_017::test_fn()) as i64);
println!("expression_statements__nonunit_calls_and_pure_expressions_can_be_discarded:0={}", (legacy_calls::case_018::test_fn()) as i64);
println!("expression_statements__unit_values_can_be_bound_and_returned_from_unit_functions:0={}", { legacy_calls::case_019::test_fn(); 0i64 });
println!("u64_ops__u64_calls_and_mixed_type_errors_are_checked:0={}", (legacy_calls::case_020::test_fn()) as i64);
println!("function_visibility__supported_visibilities_do_not_change_function_names_or_indices:0={}", (legacy_calls::case_021::test_fn()) as i64);
println!("function_visibility__visibility_preserves_forward_calls_and_native_symbols:0={}", (legacy_calls::case_022::test_fn()) as i64);
println!("redundant_moves__calls_keep_only_required_argument_and_result_moves:0={}", (legacy_calls::case_023::test_fn()) as i64);
println!("casts__casts_work_in_calls_and_ranges:0={}", (legacy_calls::case_024::test_fn()) as i64);
println!("function_calls__zero_argument_calls_emit_symbols_and_relocations:0={}", (legacy_calls::case_025::test_fn()) as i64);
},
_ => std::process::exit(2),
}
}
