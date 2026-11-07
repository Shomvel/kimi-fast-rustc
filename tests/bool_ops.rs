use kimi_fast_rustc::x86::MachineFunction;
use kimi_fast_rustc::{frontend, machine_eval, native, native_program, placement};

fn compile(source: &str) -> MachineFunction {
    let graph = frontend::parse(source).unwrap();
    graph.validate().unwrap();
    let graph = placement::place_for_native(&graph).unwrap();
    native::compile_graph(&graph, "main").unwrap()
}

#[test]
fn comparisons_and_logical_not_return_bool() {
    let function =
        compile("fn main(value: i64) -> bool { let positive = value > 0; return !positive; }");
    for (value, expected) in [(-1, 1), (0, 1), (1, 0), (42, 0)] {
        assert_eq!(machine_eval::evaluate(&function, &[value]), Ok(expected));
    }
}

#[test]
fn bool_parameters_and_logical_operators_cover_truth_tables() {
    let and = compile("fn main(a: bool, b: bool) -> bool { return a && b; }");
    let or = compile("fn main(a: bool, b: bool) -> bool { return a || b; }");
    for (a, b, expected_and, expected_or) in
        [(0, 0, 0, 0), (0, 1, 0, 1), (1, 0, 0, 1), (1, 1, 1, 1)]
    {
        assert_eq!(machine_eval::evaluate(&and, &[a, b]), Ok(expected_and));
        assert_eq!(machine_eval::evaluate(&or, &[a, b]), Ok(expected_or));
    }
}

#[test]
fn logical_precedence_supports_not_and_parentheses() {
    let function = compile("fn main(a: bool, b: bool, c: bool) -> bool { return a || b && !c; }");
    for (a, b, c, expected) in [
        (0, 0, 0, 0),
        (0, 1, 0, 1),
        (0, 1, 1, 0),
        (1, 0, 1, 1),
        (1, 1, 1, 1),
    ] {
        assert_eq!(machine_eval::evaluate(&function, &[a, b, c]), Ok(expected));
    }
}

#[test]
fn short_circuit_rhs_does_not_execute_division() {
    let and =
        compile("fn main(flag: bool) -> bool { let zero = 0; return flag && 1 / zero == 0; }");
    assert_eq!(machine_eval::evaluate(&and, &[0]), Ok(0));
    assert_eq!(
        machine_eval::evaluate(&and, &[1]),
        Err(machine_eval::MachineEvalError::DivisionByZero)
    );

    let or = compile("fn main(flag: bool) -> bool { let zero = 0; return flag || 1 / zero == 0; }");
    assert_eq!(machine_eval::evaluate(&or, &[1]), Ok(1));
    assert_eq!(
        machine_eval::evaluate(&or, &[0]),
        Err(machine_eval::MachineEvalError::DivisionByZero)
    );
}

#[test]
fn bool_values_survive_loops_and_if_expressions() {
    let function = compile(
        "fn main() -> i64 { let mut count = 0; let mut done = false; while !done && count < 4 { count += 1; if count == 4 { done = true; } } return if done && count == 4 { 42 } else { 0 }; }",
    );
    assert_eq!(machine_eval::evaluate(&function, &[]), Ok(42));
}

#[test]
fn bool_calls_use_function_signatures() {
    let graph = frontend::parse(
        "fn positive(value: i64) -> bool { return value > 0; } fn main() -> i64 { return if positive(42) && !false { 42 } else { 0 }; }",
    )
    .unwrap();
    let assembly = native_program::compile_graph_to_assembly(&graph).unwrap();
    assert!(assembly.contains("\tcall positive\n"));
}

#[test]
fn mixed_types_and_non_bool_conditions_are_rejected() {
    for source in [
        "fn main() { if 1 { let x = 2; } }",
        "fn main() -> bool { return 1; }",
        "fn main() -> i64 { return true; }",
        "fn main() -> i64 { return true + 1; }",
        "fn main() { let value: bool = 1; }",
        "fn main() { let mut value = true; value = 1; }",
        "fn main() { while 1 { let x = 2; } }",
        "fn id(value: i64) -> i64 { return value; } fn main() -> i64 { return id(true); }",
        "fn unit() {} fn main() -> i64 { return unit(); }",
    ] {
        let error = frontend::parse(source).unwrap_err();
        assert!(
            error.message.contains("type mismatch")
                || error.message.contains("expected integer operands")
                || error.message.contains("unit-returning"),
            "source: {source}; error: {error}"
        );
    }
}
