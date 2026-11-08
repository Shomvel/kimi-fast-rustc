use kimi_fast_rustc::son::{CompareOp, Graph, NodeKind};
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

fn returned_comparison(graph: &Graph) -> CompareOp {
    let result = graph
        .iter()
        .find(|(_, node)| node.kind == NodeKind::Return)
        .unwrap()
        .1;
    let comparison = graph.node(result.data[0]).unwrap();
    let NodeKind::Compare(operation) = comparison.kind else {
        panic!("expected returned comparison, got {comparison:?}")
    };
    operation
}

#[test]
fn signed_negated_comparisons_use_their_inverse() {
    for (source, inverse, expected) in [
        ("<", CompareOp::Ge, [0, 1, 1]),
        ("<=", CompareOp::Gt, [0, 0, 1]),
        (">", CompareOp::Le, [1, 1, 0]),
        (">=", CompareOp::Lt, [1, 0, 0]),
        ("==", CompareOp::Ne, [1, 0, 1]),
        ("!=", CompareOp::Eq, [0, 1, 0]),
    ] {
        let (graph, function) = optimize(&format!(
            "fn main(a: i64, b: i64) -> bool {{ return !(a {source} b); }}"
        ));
        assert_eq!(returned_comparison(&graph), inverse);
        for (a, b, expected) in [
            (3, 4, expected[0]),
            (4, 4, expected[1]),
            (5, 4, expected[2]),
        ] {
            assert_eq!(
                machine_eval::evaluate(&function, &[a, b]),
                Ok(expected),
                "operator: {source}; a: {a}; b: {b}"
            );
        }
    }
}

#[test]
fn unsigned_negated_comparisons_use_their_inverse() {
    for (source, inverse) in [
        ("<", CompareOp::UGe),
        ("<=", CompareOp::UGt),
        (">", CompareOp::ULe),
        (">=", CompareOp::ULt),
    ] {
        let (graph, function) = optimize(&format!(
            "fn main(a: u64, b: u64) -> bool {{ return !(a {source} b); }}"
        ));
        assert_eq!(returned_comparison(&graph), inverse);
        for (a, b) in [(0, 1), (-1, 0), (-1, -1), (i64::MIN, 1)] {
            let expected = i64::from(match source {
                "<" => (a as u64) >= b as u64,
                "<=" => (a as u64) > b as u64,
                ">" => (a as u64) <= b as u64,
                ">=" => (a as u64) < b as u64,
                _ => unreachable!(),
            });
            assert_eq!(machine_eval::evaluate(&function, &[a, b]), Ok(expected));
        }
    }
}

#[test]
fn comparison_with_true_or_false_removes_the_outer_comparison() {
    for suffix in ["== true", "!= false"] {
        let (graph, function) = optimize(&format!(
            "fn main(a: i64, b: i64) -> bool {{ return (a < b) {suffix}; }}"
        ));
        assert_eq!(returned_comparison(&graph), CompareOp::Lt);
        assert_eq!(machine_eval::evaluate(&function, &[3, 4]), Ok(1));
        assert_eq!(machine_eval::evaluate(&function, &[4, 3]), Ok(0));
    }
}

#[test]
fn negated_branch_comparison_does_not_materialize_two_booleans() {
    let (_, function) =
        optimize("fn main(a: i64) -> i64 { return if !(a < 4) { 42 } else { 0 }; }");
    let assembly = function.emit_gnu_assembly().unwrap();
    assert!(!assembly.contains("sete"), "{assembly}");
    assert!(!assembly.contains("setne"), "{assembly}");
    assert_eq!(machine_eval::evaluate(&function, &[3]), Ok(0));
    assert_eq!(machine_eval::evaluate(&function, &[4]), Ok(42));
    assert_eq!(machine_eval::evaluate(&function, &[5]), Ok(42));
}
