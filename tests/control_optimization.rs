use kimi_fast_rustc::son::{Graph, NodeKind};
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
fn if_branch_constants_fold_in_place() {
    let (graph, function, stats) =
        optimize("fn main(value: i64) -> i64 { return if value > 0 { 40 + 2 } else { 0 - 7 }; }");
    assert!(stats.constants_folded >= 2);
    assert!(
        graph
            .iter()
            .any(|(_, node)| node.kind == NodeKind::Const(42))
    );
    assert!(
        graph
            .iter()
            .any(|(_, node)| node.kind == NodeKind::Const(-7))
    );
    assert_eq!(machine_eval::evaluate(&function, &[1]), Ok(42));
    assert_eq!(machine_eval::evaluate(&function, &[0]), Ok(-7));
}

#[test]
fn pure_expressions_are_value_numbered_across_regions() {
    let (_, function, stats) = optimize(
        "fn main(value: i64, flag: bool) -> i64 { let next = value + 1; return if flag { next } else { value + 1 }; }",
    );
    assert!(stats.value_numbering_merges >= 1);
    assert_eq!(machine_eval::evaluate(&function, &[41, 0]), Ok(42));
    assert_eq!(machine_eval::evaluate(&function, &[41, 1]), Ok(42));
}

#[test]
fn loop_conditions_fold_without_changing_control_nodes() {
    let (graph, function, stats) = optimize(
        "fn main() -> i64 { let mut i = 0; while 1 + 1 == 2 { i += 1; if i == 3 { break; } } return i + 39; }",
    );
    assert!(stats.constants_folded >= 1);
    assert!(
        graph
            .iter()
            .any(|(_, node)| matches!(node.kind, NodeKind::If))
    );
    assert_eq!(machine_eval::evaluate(&function, &[]), Ok(42));
}

#[test]
fn trapping_division_is_not_folded_or_removed() {
    let (graph, function, _) = optimize(
        "fn main(flag: bool) -> i64 { let zero = 0; return if flag { 1 / zero } else { 2 }; }",
    );
    assert!(
        graph
            .iter()
            .any(|(_, node)| matches!(node.kind, NodeKind::Binary(_)))
    );
    assert_eq!(
        machine_eval::evaluate(&function, &[1]),
        Err(machine_eval::MachineEvalError::DivisionByZero)
    );
    assert_eq!(machine_eval::evaluate(&function, &[0]), Ok(2));
}
