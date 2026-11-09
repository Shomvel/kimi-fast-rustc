extern crate legacy_examples;
fn main() {
let selected = std::env::args().nth(1);
match selected.as_deref() {
None => {
println!("example__hello:0={}", { legacy_examples::case_000::test_fn(); 0i64 });
println!("example__integer_syntax:0={}", (legacy_examples::case_001::test_fn()) as i64);
println!("example__native_bitwise:0={}", (legacy_examples::case_002::test_fn()) as i64);
println!("example__native_bool:0={}", (legacy_examples::case_003::test_fn()) as i64);
println!("example__native_bool_compare:0={}", (legacy_examples::case_004::test_fn()) as i64);
println!("example__native_boolean_inverse:0={}", (legacy_examples::case_005::test_fn()) as i64);
println!("example__native_break_continue:0={}", (legacy_examples::case_006::test_fn()) as i64);
println!("example__native_call:0={}", (legacy_examples::case_007::test_fn()) as i64);
println!("example__native_cast:0={}", (legacy_examples::case_008::test_fn()) as i64);
println!("example__native_compare_immediate:0={}", (legacy_examples::case_009::test_fn()) as i64);
println!("example__native_const_expression:0={}", (legacy_examples::case_010::test_fn()) as i64);
println!("example__native_const_fn:0={}", (legacy_examples::case_011::test_fn()) as i64);
println!("example__native_const_item:0={}", (legacy_examples::case_012::test_fn()) as i64);
println!("example__native_early_return:0={}", (legacy_examples::case_013::test_fn()) as i64);
println!("example__native_else_if:0={}", (legacy_examples::case_014::test_fn()) as i64);
println!("example__native_expressions:0={}", (legacy_examples::case_015::test_fn()) as i64);
println!("example__native_for:0={}", (legacy_examples::case_016::test_fn()) as i64);
println!("example__native_id:0={}", (legacy_examples::case_017::test_fn()) as i64);
println!("example__native_if:0={}", (legacy_examples::case_018::test_fn()) as i64);
println!("example__native_inclusive:0={}", (legacy_examples::case_019::test_fn()) as i64);
println!("example__native_loop_value:0={}", (legacy_examples::case_020::test_fn()) as i64);
println!("example__native_main:0={}", (legacy_examples::case_021::test_fn()) as i64);
println!("example__native_match:0={}", (legacy_examples::case_022::test_fn()) as i64);
println!("example__native_match_range:0={}", (legacy_examples::case_023::test_fn()) as i64);
println!("example__native_nested_while:0={}", (legacy_examples::case_024::test_fn()) as i64);
println!("example__native_not:0={}", (legacy_examples::case_025::test_fn()) as i64);
println!("example__native_recursion:0={}", (legacy_examples::case_026::test_fn()) as i64);
println!("example__native_remainder:0={}", (legacy_examples::case_027::test_fn()) as i64);
println!("example__native_second:0={}", (legacy_examples::case_028::test_fn()) as i64);
println!("example__native_shadowing:0={}", (legacy_examples::case_029::test_fn()) as i64);
println!("example__native_signed_division:0={}", (legacy_examples::case_030::test_fn()) as i64);
println!("example__native_statement_if:0={}", (legacy_examples::case_031::test_fn()) as i64);
println!("example__native_strength:0={}", (legacy_examples::case_032::test_fn()) as i64);
println!("example__native_sum8:0={}", (legacy_examples::case_033::test_fn()) as i64);
println!("example__native_tail:0={}", (legacy_examples::case_034::test_fn()) as i64);
println!("example__native_u32_immediate:0={}", (legacy_examples::case_035::test_fn()) as i64);
println!("example__native_u64:0={}", (legacy_examples::case_036::test_fn()) as i64);
println!("example__native_unary_strength:0={}", (legacy_examples::case_037::test_fn()) as i64);
println!("example__native_unit:0={}", { legacy_examples::case_038::test_fn(); 0i64 });
println!("example__native_visibility:0={}", (legacy_examples::case_039::test_fn()) as i64);
println!("example__native_while:0={}", (legacy_examples::case_040::test_fn()) as i64);
},
_ => std::process::exit(2),
}
}
