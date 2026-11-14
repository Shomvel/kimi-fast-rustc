extern crate legacy_loops;
fn main() {
let selected = std::env::args().nth(1);
match selected.as_deref() {
None => {
println!("loops__loop_phi_copies_preserve_sequential_assignments:0={}", (legacy_loops::case_000::test_fn()) as i64);
println!("loop_control__break_preserves_values_at_the_exit_edge:0={}", (legacy_loops::case_001::test_fn()) as i64);
println!("loop_control__continue_skips_only_the_rest_of_the_current_iteration:0={}", (legacy_loops::case_002::test_fn()) as i64);
println!("loop_control__one_terminal_if_arm_still_merges_the_reachable_arm:0={}", (legacy_loops::case_003::test_fn()) as i64);
println!("loop_control__continue_and_break_can_terminate_different_if_arms:0={}", (legacy_loops::case_004::test_fn()) as i64);
println!("loop_control__nested_loop_control_targets_the_innermost_loop:0={}", (legacy_loops::case_005::test_fn()) as i64);
println!("loop_control__break_exit_phi_preserves_bool_and_integer_types:0={}", (legacy_loops::case_006::test_fn()) as i64);
println!("loop_values__loop_expression_produces_the_break_value:0={}", (legacy_loops::case_007::test_fn()) as i64);
println!("loop_values__loop_expression_can_be_the_function_tail:0={}", (legacy_loops::case_008::test_fn()) as i64);
println!("loop_values__conditional_break_and_continue_preserve_exit_snapshots:0={}", (legacy_loops::case_009::test_fn()) as i64);
println!("loop_values__nested_loop_expressions_target_the_innermost_loop:0={}", (legacy_loops::case_010::test_fn()) as i64);
println!("loop_values__loop_result_respects_shadowing_and_restores_the_outer_binding:0={}", (legacy_loops::case_011::test_fn()) as i64);
println!("early_return__return_inside_loop_leaves_without_updating_loop_phis:0={}", (legacy_loops::case_012::test_fn()) as i64);
println!("statement_if__if_inside_loop_accumulates_selected_values:0={}", (legacy_loops::case_013::test_fn()) as i64);
println!("statement_if__branch_with_inner_loop_jumps_to_the_outer_merge:0={}", (legacy_loops::case_014::test_fn()) as i64);
println!("for_loops__exclusive_ranges_accumulate_and_empty_ranges_do_not_run:0={}", (legacy_loops::case_015::test_fn()) as i64);
println!("for_loops__continue_advances_the_range_index:0={}", (legacy_loops::case_016::test_fn()) as i64);
println!("for_loops__break_exits_the_range_loop:0={}", (legacy_loops::case_017::test_fn()) as i64);
println!("for_loops__nested_ranges_target_the_inner_loop_control_context:0={}", (legacy_loops::case_018::test_fn()) as i64);
println!("for_loops__early_return_works_inside_a_range_loop:0={}", (legacy_loops::case_019::test_fn()) as i64);
println!("inclusive_ranges__inclusive_ranges_include_the_end_and_skip_empty_ranges:0={}", (legacy_loops::case_020::test_fn()) as i64);
println!("inclusive_ranges__continue_on_the_last_inclusive_iteration_exits_cleanly:0={}", (legacy_loops::case_021::test_fn()) as i64);
println!("inclusive_ranges__inclusive_ranges_support_early_return:0={}", (legacy_loops::case_022::test_fn()) as i64);
println!("bool_comparisons__bool_comparisons_accept_loop_phi_values:0={}", (legacy_loops::case_023::test_fn()) as i64);
println!("bool_ops__bool_values_survive_loops_and_if_expressions:0={}", (legacy_loops::case_024::test_fn()) as i64);
println!("assignment_syntax__compound_assignments_work_inside_loops:0={}", (legacy_loops::case_025::test_fn()) as i64);
println!("shadowing__loop_shadows_do_not_leak_into_back_edges_or_breaks:0={}", (legacy_loops::case_026::test_fn()) as i64);
println!("shadowing__for_variables_shadow_outer_bindings_without_losing_outer_mutability:0={}", (legacy_loops::case_027::test_fn()) as i64);
println!("control_optimization__loop_conditions_fold_without_changing_control_nodes:0={}", (legacy_loops::case_028::test_fn()) as i64);
println!("loops__mutable_accumulator_runs_zero_or_many_iterations:0={}", (legacy_loops::case_029::test_fn(std::hint::black_box((0i64)))) as i64);
println!("loops__mutable_accumulator_runs_zero_or_many_iterations:1={}", (legacy_loops::case_029::test_fn(std::hint::black_box((1i64)))) as i64);
println!("loops__mutable_accumulator_runs_zero_or_many_iterations:2={}", (legacy_loops::case_029::test_fn(std::hint::black_box((2i64)))) as i64);
println!("loops__mutable_accumulator_runs_zero_or_many_iterations:3={}", (legacy_loops::case_029::test_fn(std::hint::black_box((10i64)))) as i64);
println!("loops__mutable_accumulator_runs_zero_or_many_iterations:4={}", (legacy_loops::case_029::test_fn(std::hint::black_box((100i64)))) as i64);
println!("loops__nested_loops_and_body_bindings_preserve_values:0={}", (legacy_loops::case_030::test_fn(std::hint::black_box((0i64)))) as i64);
println!("loops__nested_loops_and_body_bindings_preserve_values:1={}", (legacy_loops::case_030::test_fn(std::hint::black_box((1i64)))) as i64);
println!("loops__nested_loops_and_body_bindings_preserve_values:2={}", (legacy_loops::case_030::test_fn(std::hint::black_box((2i64)))) as i64);
println!("loops__nested_loops_and_body_bindings_preserve_values:3={}", (legacy_loops::case_030::test_fn(std::hint::black_box((4i64)))) as i64);
println!("loops__nested_loops_and_body_bindings_preserve_values:4={}", (legacy_loops::case_030::test_fn(std::hint::black_box((8i64)))) as i64);
println!("loops__loop_survives_register_pressure:0={}", (legacy_loops::case_031::test_fn(std::hint::black_box((1i64)), std::hint::black_box((2i64)), std::hint::black_box((3i64)), std::hint::black_box((4i64)), std::hint::black_box((5i64)), std::hint::black_box((6i64)), std::hint::black_box((7i64)), std::hint::black_box((8i64)), std::hint::black_box((9i64)), std::hint::black_box((10i64)), std::hint::black_box((11i64)), std::hint::black_box((12i64)))) as i64);
println!("for_loops__usize_is_a_u64_alias_in_ranges_and_bindings:0={}", (legacy_loops::case_032::test_fn()) as i64);
println!("inclusive_ranges__inclusive_u64_max_does_not_wrap_to_zero__0:0={}", (legacy_loops::case_033::test_fn()) as i64);
println!("inclusive_ranges__inclusive_u64_max_does_not_wrap_to_zero__1:0={}", (legacy_loops::case_034::test_fn()) as i64);
println!("loop_values__break_literals_unify_with_other_breaks_and_the_expected_type:0={}", (legacy_loops::case_035::test_fn(std::hint::black_box(true))) as i64);
println!("loop_values__break_literals_unify_with_other_breaks_and_the_expected_type:1={}", (legacy_loops::case_035::test_fn(std::hint::black_box(false))) as i64);
println!("loop_values__nested_immediate_loops_preserve_result_types:0={}", (legacy_loops::case_036::test_fn()) as i64);
println!("loop_values__unit_loop_expressions_allow_break_without_a_value:0={}", { legacy_loops::case_037::test_fn(); 0i64 });
println!("loop_invariant__pure_expressions_hoist_and_match_baseline_for_zero_or_many_iterations:0={}", (legacy_loops::case_038::test_fn(std::hint::black_box((3i64)), std::hint::black_box((4i64)), std::hint::black_box((0i64)))) as i64);
println!("loop_invariant__pure_expressions_hoist_and_match_baseline_for_zero_or_many_iterations:1={}", (legacy_loops::case_038::test_fn(std::hint::black_box((3i64)), std::hint::black_box((4i64)), std::hint::black_box((1i64)))) as i64);
println!("loop_invariant__pure_expressions_hoist_and_match_baseline_for_zero_or_many_iterations:2={}", (legacy_loops::case_038::test_fn(std::hint::black_box((3i64)), std::hint::black_box((4i64)), std::hint::black_box((5i64)))) as i64);
println!("loop_invariant__pure_expressions_hoist_and_match_baseline_for_zero_or_many_iterations:3={}", (legacy_loops::case_038::test_fn(std::hint::black_box((9i64)), std::hint::black_box((2i64)), std::hint::black_box((4i64)))) as i64);
println!("loop_invariant__pure_expressions_hoist_and_match_baseline_for_zero_or_many_iterations:4={}", (legacy_loops::case_038::test_fn(std::hint::black_box((9223372036854775807i64)), std::hint::black_box((2i64)), std::hint::black_box((3i64)))) as i64);
println!("loop_invariant__pure_expressions_hoist_and_match_baseline_for_zero_or_many_iterations:5={}", (legacy_loops::case_038::test_fn(std::hint::black_box((-9223372036854775808i64)), std::hint::black_box((-1i64)), std::hint::black_box((2i64)))) as i64);
println!("loop_invariant__nested_loops_hoist_only_past_loops_whose_phis_are_not_inputs:0={}", (legacy_loops::case_039::test_fn(std::hint::black_box((2i64)), std::hint::black_box((3i64)), std::hint::black_box((0i64)), std::hint::black_box((4i64)))) as i64);
println!("loop_invariant__nested_loops_hoist_only_past_loops_whose_phis_are_not_inputs:1={}", (legacy_loops::case_039::test_fn(std::hint::black_box((2i64)), std::hint::black_box((3i64)), std::hint::black_box((1i64)), std::hint::black_box((0i64)))) as i64);
println!("loop_invariant__nested_loops_hoist_only_past_loops_whose_phis_are_not_inputs:2={}", (legacy_loops::case_039::test_fn(std::hint::black_box((2i64)), std::hint::black_box((3i64)), std::hint::black_box((1i64)), std::hint::black_box((4i64)))) as i64);
println!("loop_invariant__nested_loops_hoist_only_past_loops_whose_phis_are_not_inputs:3={}", (legacy_loops::case_039::test_fn(std::hint::black_box((2i64)), std::hint::black_box((3i64)), std::hint::black_box((3i64)), std::hint::black_box((4i64)))) as i64);
println!("loop_invariant__nested_loops_hoist_only_past_loops_whose_phis_are_not_inputs:4={}", (legacy_loops::case_039::test_fn(std::hint::black_box((5i64)), std::hint::black_box((-2i64)), std::hint::black_box((3i64)), std::hint::black_box((2i64)))) as i64);
println!("loop_invariant__trapping_division_is_not_hoisted_or_removed:0={}", (legacy_loops::case_040::test_fn(std::hint::black_box((7i64)), std::hint::black_box((0i64)), std::hint::black_box((0i64)))) as i64);
println!("loop_invariant__trapping_division_is_not_hoisted_or_removed:1={}", (legacy_loops::case_040::test_fn(std::hint::black_box((-9223372036854775808i64)), std::hint::black_box((-1i64)), std::hint::black_box((0i64)))) as i64);
println!("loop_invariant__trapping_division_is_not_hoisted_or_removed:2={}", (legacy_loops::case_040::test_fn(std::hint::black_box((9i64)), std::hint::black_box((2i64)), std::hint::black_box((4i64)))) as i64);
println!("loop_and_tail__unconditional_loop_supports_break_and_continue:0={}", (legacy_loops::case_041::test_fn()) as i64);
println!("bool_optimizer_regression__loop_exit_expressions_use_current_phi_values__0:0={}", (legacy_loops::case_042::test_fn()) as i64);
println!("bool_optimizer_regression__loop_exit_expressions_use_current_phi_values__1:0={}", (legacy_loops::case_043::test_fn()) as i64);
println!("bool_optimizer_regression__loop_exit_expressions_use_current_phi_values__2:0={}", (legacy_loops::case_044::test_fn()) as i64);
println!("shadowing__loop_shadows_do_not_leak_into_back_edges_or_breaks__break:0={}", (legacy_loops::case_045::test_fn()) as i64);
println!("shadowing__for_variables_shadow_outer_bindings_without_losing_outer_mutability__range_bound:0={}", (legacy_loops::case_046::test_fn()) as i64);
println!("loop_values__break_literals_unify_with_other_breaks_and_the_expected_type__tail:0={}", (legacy_loops::case_047::test_fn()) as i64);
println!("loop_values__unit_loop_expressions_allow_break_without_a_value__tail:0={}", { legacy_loops::case_048::test_fn(); 0i64 });
},
Some("loop_invariant__trapping_division_is_not_hoisted_or_removed:3") => { println!("loop_invariant__trapping_division_is_not_hoisted_or_removed:3={}", (legacy_loops::case_040::test_fn(std::hint::black_box((7i64)), std::hint::black_box((0i64)), std::hint::black_box((1i64)))) as i64); },
Some("loop_invariant__trapping_division_is_not_hoisted_or_removed:4") => { println!("loop_invariant__trapping_division_is_not_hoisted_or_removed:4={}", (legacy_loops::case_040::test_fn(std::hint::black_box((-9223372036854775808i64)), std::hint::black_box((-1i64)), std::hint::black_box((1i64)))) as i64); },
_ => std::process::exit(2),
}
}
