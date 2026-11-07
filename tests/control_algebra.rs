use kimi_fast_rustc::son::{BinaryOp, Graph, NodeKind};
use kimi_fast_rustc::x86::MachineFunction;
use kimi_fast_rustc::{frontend, machine_eval, native, optimizer_pipeline, placement};

fn optimize(source: &str) -> (Graph, MachineFunction, optimizer_pipeline::OptimizeStats) {
    let mut graph = frontend::parse(source).unwrap();
    let stats = optimizer_pipeline::optimize(&mut graph);
    graph.validate().unwrap();
    let placed = placement::place_for_native(&graph).unwrap();
    let function = native::compile_graph(&placed, "main").unwrap();
    (graph, function, stats)
}

#[test]
fn additive_and_multiplicative_identities_are_removed() {
    let (graph, function, stats) = optimize(
        "fn main(value: i64, flag: bool) -> i64 { return if flag { value + 0 } else { value * 1 }; }",
    );
    assert!(stats.algebraic_simplifications >= 2);
    assert!(
        !graph
            .iter()
            .any(|(_, node)| matches!(node.kind, NodeKind::Binary(BinaryOp::Add | BinaryOp::Mul)))
    );
    assert_eq!(machine_eval::evaluate(&function, &[42, 0]), Ok(42));
    assert_eq!(machine_eval::evaluate(&function, &[42, 1]), Ok(42));
}

#[test]
fn zero_and_self_operations_become_constants() {
    let (graph, function, stats) = optimize(
        "fn main(value: i64, flag: bool) -> i64 { let zero = value & 0; return if flag { zero } else { value ^ value }; }",
    );
    assert!(stats.algebraic_simplifications >= 2);
    assert!(
        graph
            .iter()
            .any(|(_, node)| node.kind == NodeKind::Const(0))
    );
    assert_eq!(machine_eval::evaluate(&function, &[123, 0]), Ok(0));
    assert_eq!(machine_eval::evaluate(&function, &[123, 1]), Ok(0));
}

#[test]
fn self_comparison_simplifies_but_self_division_still_traps() {
    let (graph, function, stats) = optimize(
        "fn main(value: i64, flag: bool) -> i64 { let same = value == value; return if flag { same as i64 } else { value / value }; }",
    );
    assert!(stats.algebraic_simplifications >= 1);
    assert_eq!(machine_eval::evaluate(&function, &[0, 1]), Ok(1));
    assert_eq!(
        machine_eval::evaluate(&function, &[0, 0]),
        Err(machine_eval::MachineEvalError::DivisionByZero)
    );
    assert_eq!(machine_eval::evaluate(&function, &[2, 0]), Ok(1));
    assert!(
        graph
            .iter()
            .any(|(_, node)| node.kind == NodeKind::Const(1))
    );
}

#[test]
fn unsigned_division_and_remainder_by_one_are_simplified() {
    let (_, function, stats) = optimize(
        "fn main(value: u64, flag: bool) -> u64 { return if flag { value / 1 } else { value % 1 }; }",
    );
    assert!(stats.algebraic_simplifications >= 2);
    assert_eq!(machine_eval::evaluate(&function, &[-1, 1]), Ok(-1));
    assert_eq!(machine_eval::evaluate(&function, &[-1, 0]), Ok(0));
}
