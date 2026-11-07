use kimi_fast_rustc::x86::MachineFunction;
use kimi_fast_rustc::{frontend, machine_eval, native, native_program, placement};

fn compile(source: &str) -> MachineFunction {
    let graph = frontend::parse(source).unwrap();
    graph.validate().unwrap();
    let graph = placement::place_for_native(&graph).unwrap();
    native::compile_graph(&graph, "main").unwrap()
}

#[test]
fn unconditional_loop_supports_break_and_continue() {
    let function = compile(
        "fn main() -> i64 { let mut sum = 0; let mut i = 0; loop { i += 1; if i > 10 { break; } if i % 2 == 1 { continue; } sum += i; } return sum + 12; }",
    );
    assert_eq!(machine_eval::evaluate(&function, &[]), Ok(42));
}

#[test]
fn tail_literals_variables_and_bools_do_not_need_return() {
    let literal = compile("fn main() -> i64 { 42 }");
    assert_eq!(machine_eval::evaluate(&literal, &[]), Ok(42));

    let variable = compile("fn main(value: i64) -> i64 { let result = value + 1; result }");
    assert_eq!(machine_eval::evaluate(&variable, &[41]), Ok(42));

    let boolean = compile("fn main(value: i64) -> bool { value > 0 }");
    assert_eq!(machine_eval::evaluate(&boolean, &[1]), Ok(1));
    assert_eq!(machine_eval::evaluate(&boolean, &[0]), Ok(0));
}

#[test]
fn tail_expressions_support_casts_and_arithmetic() {
    let function = compile("fn main(flag: bool) -> i64 { flag as i64 + 41 }");
    assert_eq!(machine_eval::evaluate(&function, &[0]), Ok(41));
    assert_eq!(machine_eval::evaluate(&function, &[1]), Ok(42));
}

#[test]
fn tail_calls_work_after_a_range_loop() {
    let graph = frontend::parse(
        "fn sum() -> i64 { let mut total = 0; for i in 0..5 { total += i; } total + 32 } fn main() -> i64 { sum() }",
    )
    .unwrap();
    let assembly = native_program::compile_graph_to_assembly(&graph).unwrap();
    assert!(assembly.contains("\tcall sum\n"));
}

#[test]
fn tail_type_mismatch_is_still_rejected() {
    let error = frontend::parse("fn main() -> bool { 1 }").unwrap_err();
    assert!(error.message.contains("type mismatch"));
}
