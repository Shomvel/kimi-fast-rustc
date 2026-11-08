use kimi_fast_rustc::son::{CompareOp, Graph, NodeKind};
use kimi_fast_rustc::x86::{MachineFunction, MachineInstr, Operand};
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
    let NodeKind::Compare(operation) = graph.node(result.data[0]).unwrap().kind else {
        panic!("expected comparison")
    };
    operation
}

#[test]
fn signed_left_constants_swap_to_the_inverse_right_immediate_comparison() {
    for (source, inverse, expected) in [
        ("<", CompareOp::Gt, [0, 0, 1]),
        ("<=", CompareOp::Ge, [0, 1, 1]),
        (">", CompareOp::Lt, [1, 0, 0]),
        (">=", CompareOp::Le, [1, 1, 0]),
        ("==", CompareOp::Eq, [0, 1, 0]),
        ("!=", CompareOp::Ne, [1, 0, 1]),
    ] {
        let (graph, function) = optimize(&format!(
            "fn main(value: i64) -> bool {{ return 2 {source} value; }}"
        ));
        assert_eq!(returned_comparison(&graph), inverse);
        for (value, expected) in [(1, expected[0]), (2, expected[1]), (3, expected[2])] {
            assert_eq!(
                machine_eval::evaluate(&function, &[value]),
                Ok(expected),
                "operator: {source}; value: {value}"
            );
        }
    }
}

#[test]
fn unsigned_left_constants_swap_to_the_inverse_right_immediate_comparison() {
    for (source, inverse) in [
        ("<", CompareOp::UGt),
        ("<=", CompareOp::UGe),
        (">", CompareOp::ULt),
        (">=", CompareOp::ULe),
    ] {
        let (graph, function) = optimize(&format!(
            "fn main(value: u64) -> bool {{ return 2 {source} value; }}"
        ));
        assert_eq!(returned_comparison(&graph), inverse);
        for value in [-1, 0, 1, 2, 3, i64::MIN] {
            let expected = i64::from(match source {
                "<" => 2u64 < value as u64,
                "<=" => 2u64 <= value as u64,
                ">" => 2u64 > value as u64,
                ">=" => 2u64 >= value as u64,
                _ => unreachable!(),
            });
            assert_eq!(machine_eval::evaluate(&function, &[value]), Ok(expected));
        }
    }
}

#[test]
fn left_constants_do_not_materialize_a_register() {
    let (_, function) = optimize("fn main(value: i64) -> bool { return 2 < value; }");
    assert!(function.instructions.iter().any(|instruction| matches!(
        instruction,
        MachineInstr::Cmp {
            rhs: Operand::Imm(2),
            ..
        }
    )));
    assert!(
        !function
            .instructions
            .iter()
            .any(|instruction| matches!(instruction, MachineInstr::MovImm { value: 2, .. }))
    );
}

#[test]
fn two_constant_comparisons_still_fold() {
    let (_, function) = optimize("fn main() -> bool { return 2 < 3; }");
    assert_eq!(machine_eval::evaluate(&function, &[]), Ok(1));
}
