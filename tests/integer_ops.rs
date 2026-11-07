use kimi_fast_rustc::son::{Graph, UnaryOp};
use kimi_fast_rustc::x86::MachineFunction;
use kimi_fast_rustc::{frontend, interpreter, machine_eval, native, native_object, placement};

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
fn variable_negation_wraps_at_the_i64_boundary() {
    for value in [i64::MIN, -42, 0, 42, i64::MAX] {
        let function = compile("fn main(value: i64) -> i64 { return -value; }");
        assert_eq!(
            machine_eval::evaluate(&function, &[value]),
            Ok(value.wrapping_neg())
        );
    }
}

#[test]
fn bitwise_precedence_and_comparisons_match_the_interpreter() {
    assert_differential(
        "fn main(a: i64, b: i64) -> i64 { return a | b ^ a & b; }",
        &[[6, 3], [-8, 7]],
    );
    assert_differential(
        "fn main(a: i64, b: i64) -> i64 { return a + b << 2; }",
        &[[3, 4], [-2, 5]],
    );
    assert_differential(
        "fn main(a: i64, b: i64) -> i64 { return a | b == 7; }",
        &[[3, 4], [8, 1]],
    );
}

#[test]
fn every_bitwise_operator_matches_the_interpreter() {
    for operator in ["&", "|", "^"] {
        assert_differential(
            &format!("fn main(a: i64, b: i64) -> i64 {{ return a {operator} b; }}"),
            &[[12, 10], [-8, 3], [i64::MIN, i64::MAX]],
        );
    }
}

#[test]
fn shifts_mask_counts_and_preserve_the_sign() {
    for operator in ["<<", ">>"] {
        assert_differential(
            &format!("fn main(a: i64, b: i64) -> i64 {{ return a {operator} b; }}"),
            &[
                [1, 0],
                [1, 63],
                [1, 64],
                [1, 65],
                [-8, 1],
                [i64::MIN, 63],
                [i64::MAX, -1],
            ],
        );
    }
}

#[test]
fn shifts_survive_spills_and_loop_phi_values() {
    let function = compile(
        "fn main() -> i64 { let a = 1; let b = 2; let c = 3; let d = 4; let e = 5; let f = 6; let g = 7; let h = 8; let i = 9; let j = 10; let k = 11; let l = 12; let mut value = 3; let mut count = 0; while count < 4 { value = value << 2; count = count + 1; } return (value >> 1) + a + b + c + d + e + f + g + h + i + j + k + l; }",
    );
    assert!(function.stack_slots > 0);
    assert_eq!(machine_eval::evaluate(&function, &[]), Ok(462));
}

#[test]
fn son_bitwise_not_and_direct_objects_are_supported() {
    let mut graph = Graph::new();
    let start = graph.start();
    let zero = graph.constant(0);
    let inverted = graph.unary(UnaryOp::Not, zero);
    graph.return_(start, Some(inverted));
    let function = native::compile_graph(&graph, "main").unwrap();
    assert_eq!(machine_eval::evaluate(&function, &[]), Ok(-1));
    assert!(native_object::compile_graph_to_object(&graph, "main").is_ok());
}
