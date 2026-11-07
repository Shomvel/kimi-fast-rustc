use kimi_fast_rustc::x86::MachineFunction;
use kimi_fast_rustc::{frontend, interpreter, machine_eval, native, native_program, placement};

fn compile(source: &str) -> MachineFunction {
    let graph = frontend::parse(source).unwrap();
    graph.validate().unwrap();
    let graph = placement::place_for_native(&graph).unwrap();
    native::compile_graph(&graph, "main").unwrap()
}

fn assert_differential(source: &str, cases: &[[i64; 2]]) {
    let graph = frontend::parse(source).unwrap();
    let placed = placement::place_for_native(&graph).unwrap();
    let function = native::compile_graph(&placed, "main").unwrap();
    for arguments in cases {
        let expected = interpreter::interpret(&graph, arguments).unwrap();
        assert_eq!(
            machine_eval::evaluate(&function, arguments),
            Ok(expected),
            "source: {source}; arguments: {arguments:?}"
        );
    }
}

#[test]
fn u64_literals_and_inference_cover_the_full_bit_range() {
    let maximum = compile("fn main() -> u64 { return 18446744073709551615u64; }");
    assert_eq!(machine_eval::evaluate(&maximum, &[]), Ok(-1));

    let inferred = compile("fn main() -> u64 { let value: u64 = 42; return value; }");
    assert_eq!(machine_eval::evaluate(&inferred, &[]), Ok(42));

    let branches = compile("fn main(flag: bool) -> u64 { return if flag { 1 } else { 2u64 }; }");
    assert_eq!(machine_eval::evaluate(&branches, &[0]), Ok(2));
    assert_eq!(machine_eval::evaluate(&branches, &[1]), Ok(1));
}

#[test]
fn unsigned_comparisons_do_not_use_signed_ordering() {
    for operator in ["<", "<=", ">", ">="] {
        assert_differential(
            &format!("fn main(a: u64, b: u64) -> bool {{ return a {operator} b; }}"),
            &[[0, 1], [-1, 0], [-1, i64::MAX], [i64::MIN, 1], [-1, -1]],
        );
    }
}

#[test]
fn unsigned_division_remainder_and_shifts_match_the_interpreter() {
    for operator in ["/", "%", ">>"] {
        assert_differential(
            &format!("fn main(a: u64, b: u64) -> u64 {{ return a {operator} b; }}"),
            &[
                [-1, 2],
                [-1, 3],
                [i64::MIN, 3],
                [i64::MIN, 63],
                [-1, 64],
                [-1, 65],
                [i64::MIN, -1],
            ],
        );
    }
}

#[test]
fn unsigned_division_has_no_signed_overflow_case() {
    let function = compile("fn main(a: u64, b: u64) -> u64 { return a / b; }");
    assert_eq!(machine_eval::evaluate(&function, &[i64::MIN, -1]), Ok(0));
    assert_eq!(
        machine_eval::evaluate(&function, &[1, 0]),
        Err(machine_eval::MachineEvalError::DivisionByZero)
    );
}

#[test]
fn u64_arithmetic_compound_assignments_and_loops_preserve_bits() {
    let wrapping =
        compile("fn main() -> u64 { let mut value = 0u64; value -= 1; value += 1; return value; }");
    assert_eq!(machine_eval::evaluate(&wrapping, &[]), Ok(0));

    let compound = compile(
        "fn main() -> u64 { let mut value = 100u64; value /= 3; value %= 30; return value; }",
    );
    assert_eq!(machine_eval::evaluate(&compound, &[]), Ok(3));

    let loops = compile(
        "fn main() -> u64 { let mut value = 32u64; while value > 1 { value = value >> 1; } return value + 41; }",
    );
    assert_eq!(machine_eval::evaluate(&loops, &[]), Ok(42));
}

#[test]
fn u64_calls_and_mixed_type_errors_are_checked() {
    let graph = frontend::parse(
        "fn halve(value: u64) -> u64 { return value >> 1; } fn main() -> u64 { return halve(84u64); }",
    )
    .unwrap();
    let assembly = native_program::compile_graph_to_assembly(&graph).unwrap();
    assert!(assembly.contains("\tcall halve\n"));
    assert!(assembly.contains("\tshr "));

    for source in [
        "fn main(a: u64, b: i64) -> u64 { return a + b; }",
        "fn main() -> u64 { return -1; }",
        "fn main() -> u64 { return true; }",
        "fn main() -> i64 { return 1u64; }",
    ] {
        assert!(frontend::parse(source).is_err(), "source: {source}");
    }
}
