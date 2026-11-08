use kimi_fast_rustc::x86::MachineFunction;
use kimi_fast_rustc::{frontend, machine_eval, native, native_program, placement};

fn compile(source: &str) -> MachineFunction {
    let graph = frontend::parse(source).unwrap();
    graph.validate().unwrap();
    let graph = placement::place_for_native(&graph).unwrap();
    native::compile_graph(&graph, "main").unwrap()
}

#[test]
fn bool_equality_and_inequality_cover_the_truth_table() {
    for (operator, expected) in [("==", [1, 0, 0, 1]), ("!=", [0, 1, 1, 0])] {
        let function = compile(&format!(
            "fn main(a: bool, b: bool) -> bool {{ return a {operator} b; }}"
        ));
        for ((a, b), expected) in [(0, 0), (0, 1), (1, 0), (1, 1)].into_iter().zip(expected) {
            assert_eq!(
                machine_eval::evaluate(&function, &[a, b]),
                Ok(expected),
                "operator: {operator}; a: {a}; b: {b}"
            );
        }
    }
}

#[test]
fn bool_comparisons_preserve_short_circuit_evaluation_and_precedence() {
    let and = compile(
        "fn main(a: bool, b: bool, divisor: i64) -> bool { return a == b && 1 / divisor == 0; }",
    );
    assert_eq!(machine_eval::evaluate(&and, &[0, 1, 0]), Ok(0));
    assert_eq!(machine_eval::evaluate(&and, &[1, 0, 0]), Ok(0));
    assert_eq!(machine_eval::evaluate(&and, &[1, 1, 2]), Ok(1));
    assert_eq!(
        machine_eval::evaluate(&and, &[1, 1, 0]),
        Err(machine_eval::MachineEvalError::DivisionByZero)
    );

    let or = compile(
        "fn main(a: bool, b: bool, divisor: i64) -> bool { return a != b || 1 / divisor == 0; }",
    );
    assert_eq!(machine_eval::evaluate(&or, &[1, 0, 0]), Ok(1));
    assert_eq!(machine_eval::evaluate(&or, &[0, 1, 0]), Ok(1));
    assert_eq!(machine_eval::evaluate(&or, &[1, 1, 2]), Ok(1));
    assert_eq!(machine_eval::evaluate(&or, &[1, 1, 1]), Ok(0));
    assert_eq!(
        machine_eval::evaluate(&or, &[0, 0, 0]),
        Err(machine_eval::MachineEvalError::DivisionByZero)
    );
}

#[test]
fn bool_comparisons_accept_branch_phi_values() {
    let function = compile(
        "fn main(a: bool, b: bool) -> bool { let expression_value = if a == b { a } else { b }; let mut statement_value = false; if a != b { statement_value = true; } else { statement_value = false; } return expression_value == statement_value; }",
    );
    for (a, b, expected) in [(0, 0, 1), (0, 1, 1), (1, 0, 0), (1, 1, 0)] {
        assert_eq!(machine_eval::evaluate(&function, &[a, b]), Ok(expected));
    }
}

#[test]
fn bool_comparisons_accept_loop_phi_values() {
    let function = compile(
        "fn main() -> i64 { let mut value = false; let mut changes = 0; let mut index = 0; while index < 4 { let next = !value; if next != value { changes += 1; } value = next; index += 1; } return if value == false && changes == 4 { 42 } else { 0 }; }",
    );
    assert_eq!(machine_eval::evaluate(&function, &[]), Ok(42));
}

#[test]
fn bool_comparisons_use_function_parameter_and_return_signatures() {
    let graph = frontend::parse(
        "fn equal(a: bool, b: bool) -> bool { return a == b; } fn different(a: bool, b: bool) -> bool { return a != b; } fn main() -> i64 { return if equal(true, false) != different(false, true) { 42 } else { 0 }; }",
    )
    .unwrap();
    let program = native_program::compile_graph(&graph).unwrap();
    let equal = program
        .functions
        .iter()
        .find(|function| function.name == "equal")
        .unwrap();
    let different = program
        .functions
        .iter()
        .find(|function| function.name == "different")
        .unwrap();
    for (a, b, expected_equal) in [(0, 0, 1), (0, 1, 0), (1, 0, 0), (1, 1, 1)] {
        assert_eq!(machine_eval::evaluate(equal, &[a, b]), Ok(expected_equal));
        assert_eq!(
            machine_eval::evaluate(different, &[a, b]),
            Ok(1 - expected_equal)
        );
    }

    let assembly = program.emit_gnu_assembly().unwrap();
    assert!(assembly.contains("\tcall equal\n"));
    assert!(assembly.contains("\tcall different\n"));
    assert_eq!(&program.emit_object().unwrap()[..4], b"\x7fELF");
}

#[test]
fn integer_comparisons_keep_signed_and_unsigned_semantics() {
    let signed = compile("fn main(a: i64, b: i64) -> bool { return a < b; }");
    let unsigned = compile("fn main(a: u64, b: u64) -> bool { return a < b; }");
    for (a, b, expected_signed, expected_unsigned) in
        [(-1, 1, 1, 0), (i64::MIN, i64::MAX, 1, 0), (0, -1, 0, 1)]
    {
        assert_eq!(
            machine_eval::evaluate(&signed, &[a, b]),
            Ok(expected_signed)
        );
        assert_eq!(
            machine_eval::evaluate(&unsigned, &[a, b]),
            Ok(expected_unsigned)
        );
    }

    for ty in ["i64", "u64"] {
        let equal = compile(&format!(
            "fn main(a: {ty}, b: {ty}) -> bool {{ return a == b; }}"
        ));
        let different = compile(&format!(
            "fn main(a: {ty}, b: {ty}) -> bool {{ return a != b; }}"
        ));
        assert_eq!(machine_eval::evaluate(&equal, &[-1, -1]), Ok(1));
        assert_eq!(machine_eval::evaluate(&different, &[-1, -1]), Ok(0));
        assert_eq!(machine_eval::evaluate(&equal, &[-1, 1]), Ok(0));
        assert_eq!(machine_eval::evaluate(&different, &[-1, 1]), Ok(1));
    }
}

#[test]
fn mixed_bool_integer_unit_and_ordering_comparisons_are_rejected() {
    for (source, expected) in [
        (
            "fn main(a: bool, b: i64) -> bool { return a == b; }",
            "type mismatch: expected `bool`, found `i64`",
        ),
        (
            "fn main(a: u64, b: bool) -> bool { return a != b; }",
            "type mismatch: expected `bool`, found `u64`",
        ),
        (
            "fn unit() {} fn main(a: bool) -> bool { return a == unit(); }",
            "type mismatch: expected `bool`, found `()`",
        ),
        (
            "fn unit() {} fn main(a: i64) -> bool { return a != unit(); }",
            "type mismatch: expected `i64`, found `()`",
        ),
        (
            "fn unit() {} fn main() -> bool { return unit() == unit(); }",
            "expected integer operands, found `()`",
        ),
        (
            "fn main(a: bool, b: bool) -> bool { return a < b; }",
            "expected integer operands, found `bool`",
        ),
    ] {
        let error = frontend::parse(source).unwrap_err();
        assert!(
            error.message.contains(expected),
            "source: {source}; error: {error}"
        );
    }
}

#[test]
fn native_bool_compare_example_returns_42() {
    let mut graph = frontend::parse(include_str!("../examples/native_bool_compare.kfr")).unwrap();
    graph.validate().unwrap();
    kimi_fast_rustc::optimizer_pipeline::optimize(&mut graph);
    let graph = placement::place_for_native(&graph).unwrap();
    let function = native::compile_graph(&graph, "main").unwrap();
    assert_eq!(machine_eval::evaluate(&function, &[]), Ok(42));
}
