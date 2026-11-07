use kimi_fast_rustc::x86::MachineFunction;
use kimi_fast_rustc::{frontend, machine_eval, native, native_program, placement};

fn compile(source: &str) -> MachineFunction {
    let graph = frontend::parse(source).unwrap();
    graph.validate().unwrap();
    let graph = placement::place_for_native(&graph).unwrap();
    native::compile_graph(&graph, "main").unwrap()
}

#[test]
fn every_compound_assignment_uses_the_previous_value() {
    let function = compile(
        "fn main(value: i64) -> i64 { let mut result = value; result += 5; result -= 3; result *= 4; result /= 2; result %= 7; return result; }",
    );
    for (value, expected) in [(3, 3), (8, 6), (-4, -4)] {
        assert_eq!(machine_eval::evaluate(&function, &[value]), Ok(expected));
    }
}

#[test]
fn compound_assignments_work_inside_loops() {
    let function = compile(
        "fn main() -> i64 { let mut sum = 0; let mut i = 0; while i < 10 { i += 1; sum += i; } sum *= 2; return sum; }",
    );
    assert_eq!(machine_eval::evaluate(&function, &[]), Ok(110));
}

#[test]
fn mutable_parameters_can_be_assigned_and_loop_carried() {
    let function = compile(
        "fn main(mut value: i64) -> i64 { value += 1; let mut i = 0; while i < 3 { value *= 2; i += 1; } return value; }",
    );
    for (value, expected) in [(0, 8), (4, 40), (-2, -8)] {
        assert_eq!(machine_eval::evaluate(&function, &[value]), Ok(expected));
    }
}

#[test]
fn mutable_parameters_survive_function_call_lowering() {
    let graph = frontend::parse(
        "fn adjust(mut value: i64) -> i64 { value += 5; value *= 3; return value; } fn main() -> i64 { return adjust(16); }",
    )
    .unwrap();
    let assembly = native_program::compile_graph_to_assembly(&graph).unwrap();
    assert!(assembly.contains("\tcall adjust\n"));
}

#[test]
fn immutable_compound_assignment_and_compound_division_errors_are_preserved() {
    let error = frontend::parse("fn main() { let value = 1; value += 2; }").unwrap_err();
    assert!(error.message.contains("immutable"));

    let function = compile("fn main() -> i64 { let mut value = 1; value /= 0; return value; }");
    assert_eq!(
        machine_eval::evaluate(&function, &[]),
        Err(machine_eval::MachineEvalError::DivisionByZero)
    );
}
