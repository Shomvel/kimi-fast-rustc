use kimi_fast_rustc::son::{BinaryOp, Graph, NodeKind, UnaryOp};
use kimi_fast_rustc::x86::MachineFunction;
use kimi_fast_rustc::{frontend, machine_eval, native, optimizer_pipeline, placement};

fn optimize(source: &str) -> (Graph, MachineFunction) {
    let mut graph = frontend::parse(source).unwrap();
    optimizer_pipeline::optimize(&mut graph);
    graph.validate().unwrap();
    let placed = placement::place_for_native(&graph).unwrap();
    let function = native::compile_graph(&placed, "main").unwrap();
    (graph, function)
}

fn assert_unary(graph: &Graph, operation: UnaryOp, eliminated: BinaryOp) {
    assert!(
        graph
            .iter()
            .any(|(_, node)| { node.kind == NodeKind::Unary(operation) })
    );
    assert!(
        !graph
            .iter()
            .any(|(_, node)| { node.kind == NodeKind::Binary(eliminated) })
    );
}

#[test]
fn zero_minus_becomes_wrapping_negation() {
    let (graph, function) = optimize("fn main(value: i64) -> i64 { return 0 - value; }");
    assert_unary(&graph, UnaryOp::Neg, BinaryOp::Sub);
    for value in [i64::MIN, -42, -1, 0, 1, 42, i64::MAX] {
        assert_eq!(
            machine_eval::evaluate(&function, &[value]),
            Ok(value.wrapping_neg())
        );
    }
}

#[test]
fn multiplication_by_minus_one_becomes_negation_on_either_side() {
    for source in [
        "fn main(value: i64) -> i64 { return -1 * value; }",
        "fn main(value: u64) -> u64 { return value * 0xffff_ffff_ffff_ffffu64; }",
    ] {
        let (graph, function) = optimize(source);
        assert_unary(&graph, UnaryOp::Neg, BinaryOp::Mul);
        for value in [i64::MIN, -42, -1, 0, 1, 42, i64::MAX] {
            assert_eq!(
                machine_eval::evaluate(&function, &[value]),
                Ok(value.wrapping_neg()),
                "source: {source}; value: {value}"
            );
        }
    }
}

#[test]
fn xor_with_all_ones_becomes_bitwise_not_on_either_side() {
    for source in [
        "fn main(value: i64) -> i64 { return -1 ^ value; }",
        "fn main(value: u64) -> u64 { return value ^ 0xffff_ffff_ffff_ffffu64; }",
    ] {
        let (graph, function) = optimize(source);
        assert_unary(&graph, UnaryOp::Not, BinaryOp::Xor);
        for value in [i64::MIN, -42, -1, 0, 1, 42, i64::MAX] {
            assert_eq!(
                machine_eval::evaluate(&function, &[value]),
                Ok(!value),
                "source: {source}; value: {value}"
            );
        }
    }
}
