use kimi_fast_rustc::machine_eval::{self, MachineEvalError};
use kimi_fast_rustc::son::{BinaryOp, CompareOp, Graph, NodeId, NodeKind, UnaryOp};
use kimi_fast_rustc::x86::MachineFunction;
use kimi_fast_rustc::{frontend, loop_analysis, native, optimizer_pipeline, placement};

fn compile(graph: &Graph) -> MachineFunction {
    graph.validate().unwrap();
    let placed = placement::place_for_native(graph).unwrap();
    native::compile_graph(&placed, "main").unwrap()
}

fn optimize_and_compare(
    source: &str,
    cases: &[&[i64]],
) -> (Graph, optimizer_pipeline::OptimizeStats) {
    let original = frontend::parse(source).unwrap();
    let baseline = compile(&original);
    let mut optimized = original.clone();
    let stats = optimizer_pipeline::optimize(&mut optimized);
    optimized.validate().unwrap();
    let function = compile(&optimized);
    for arguments in cases {
        assert_eq!(
            machine_eval::evaluate(&baseline, arguments),
            machine_eval::evaluate(&function, arguments),
            "arguments: {arguments:?}"
        );
    }
    (optimized, stats)
}

fn loop_headers(graph: &Graph) -> Vec<NodeId> {
    graph
        .iter()
        .filter_map(|(id, node)| {
            (node.kind == NodeKind::Region
                && node.control.iter().any(|input| {
                    *input > id
                        && graph
                            .node(*input)
                            .is_some_and(|input| input.kind == NodeKind::Jump)
                }))
            .then_some(id)
        })
        .collect()
}

#[test]
fn pure_expressions_hoist_and_match_baseline_for_zero_or_many_iterations() {
    let (graph, stats) = optimize_and_compare(
        "fn main(left: i64, right: i64, limit: i64) -> i64 { let mut sum = 0; let mut i = 0; while i < limit { sum += left * right + !right; if left < right { sum += 1; } i += 1; } return sum; }",
        &[
            &[3, 4, 0],
            &[3, 4, 1],
            &[3, 4, 5],
            &[9, 2, 4],
            &[i64::MAX, 2, 3],
            &[i64::MIN, -1, 2],
        ],
    );

    assert!(stats.loop_invariant_hoists >= 4);
    let header = loop_headers(&graph)[0];
    assert!(
        graph
            .iter()
            .any(|(id, node)| { id < header && node.kind == NodeKind::Unary(UnaryOp::Not) })
    );
    assert!(
        graph
            .iter()
            .any(|(id, node)| { id < header && node.kind == NodeKind::Binary(BinaryOp::Mul) })
    );
    assert!(graph.iter().any(|(id, node)| {
        id < header
            && node.kind == NodeKind::Compare(CompareOp::Lt)
            && node.data.iter().all(|input| {
                matches!(
                    graph.node(*input).map(|node| node.kind),
                    Some(NodeKind::Parameter { .. })
                )
            })
    }));
}

#[test]
fn nested_loops_hoist_only_past_loops_whose_phis_are_not_inputs() {
    let (graph, stats) = optimize_and_compare(
        "fn main(left: i64, right: i64, outer_limit: i64, inner_limit: i64) -> i64 { let mut sum = 0; let mut i = 0; while i < outer_limit { let mut j = 0; while j < inner_limit { sum += left * right; sum += i * right; j += 1; } i += 1; } return sum; }",
        &[
            &[2, 3, 0, 4],
            &[2, 3, 1, 0],
            &[2, 3, 1, 4],
            &[2, 3, 3, 4],
            &[5, -2, 3, 2],
        ],
    );

    assert!(stats.loop_invariant_hoists >= 3);
    let headers = loop_headers(&graph);
    assert_eq!(headers.len(), 2);
    let outer = headers[0];
    let inner = headers[1];
    let parameter_product = graph
        .iter()
        .find(|(_, node)| {
            node.kind == NodeKind::Binary(BinaryOp::Mul)
                && node.data.iter().all(|input| {
                    matches!(
                        graph.node(*input).map(|node| node.kind),
                        Some(NodeKind::Parameter { .. })
                    )
                })
        })
        .map(|(id, _)| id)
        .unwrap();
    let phi_product = graph
        .iter()
        .find(|(_, node)| {
            node.kind == NodeKind::Binary(BinaryOp::Mul)
                && node.data.iter().any(|input| {
                    graph
                        .node(*input)
                        .is_some_and(|node| node.kind == NodeKind::Phi)
                })
        })
        .map(|(id, _)| id)
        .unwrap();
    assert!(parameter_product < outer);
    assert!(phi_product > inner);
}

#[test]
fn non_dominating_branch_value_is_not_used_as_a_hoisted_representative() {
    let mut graph = Graph::new();
    let start = graph.start();
    let condition = graph.parameter(0);
    let right = graph.parameter(1);
    let branch = graph.if_(start, condition);
    let true_projection = graph.projection(branch, 0);
    let branch_value = graph.unary(UnaryOp::Not, condition);
    let true_jump = graph.jump(true_projection);
    let false_projection = graph.projection(branch, 1);
    let false_jump = graph.jump(false_projection);
    let join = graph.region(vec![true_jump, false_jump]);
    let header = graph.region(vec![join]);
    let loop_branch = graph.if_(header, condition);
    let body = graph.projection(loop_branch, 0);
    let candidate = graph.binary(BinaryOp::Add, branch_value, right);
    let back_edge = graph.jump(body);
    graph.node_mut(header).unwrap().control.push(back_edge);
    let exit = graph.projection(loop_branch, 1);
    graph.return_(exit, Some(candidate));
    graph.validate().unwrap();
    let original = graph.clone();

    assert_eq!(loop_analysis::hoist_loop_invariants(&mut graph), 0);
    assert!(graph.iter().eq(original.iter()));
}

#[test]
fn phi_dependent_and_memory_dependent_expressions_stay_in_the_loop() {
    for with_memory in [false, true] {
        let mut graph = Graph::new();
        let start = graph.start();
        let initial = graph.parameter(0);
        let right = graph.parameter(1);
        let one = graph.constant(1);
        let header = graph.region(vec![start]);
        let index = graph.phi(vec![start], vec![initial]);
        let loop_branch = graph.if_(header, initial);
        let body = graph.projection(loop_branch, 0);
        let phi_dependent = graph.binary(BinaryOp::Mul, index, right);
        let next = graph.binary(BinaryOp::Add, index, one);
        let memory_dependent = if with_memory {
            graph.add_node_with_memory(
                NodeKind::Binary(BinaryOp::Add),
                Vec::new(),
                vec![right, right],
                vec![start],
            )
        } else {
            graph.binary(BinaryOp::Div, right, right)
        };
        let back_edge = graph.jump(body);
        graph.node_mut(header).unwrap().control.push(back_edge);
        let phi = graph.node_mut(index).unwrap();
        phi.control.push(back_edge);
        phi.data.push(next);
        let exit = graph.projection(loop_branch, 1);
        let result = graph.binary(BinaryOp::Add, phi_dependent, memory_dependent);
        graph.return_(exit, Some(result));
        graph.validate().unwrap();
        let original = graph.clone();

        assert_eq!(loop_analysis::hoist_loop_invariants(&mut graph), 0);
        assert!(graph.iter().eq(original.iter()));
    }
}

#[test]
fn trapping_division_is_not_hoisted_or_removed() {
    let source = "fn main(left: i64, divisor: i64, limit: i64) -> i64 { let mut sum = 0; let mut i = 0; while i < limit { sum += left / divisor; i += 1; } return sum; }";
    let original = frontend::parse(source).unwrap();
    let baseline = compile(&original);
    let mut optimized = original.clone();
    optimizer_pipeline::optimize(&mut optimized);
    let function = compile(&optimized);

    let header = loop_headers(&optimized)[0];
    assert!(
        optimized
            .iter()
            .any(|(id, node)| { id > header && node.kind == NodeKind::Binary(BinaryOp::Div) })
    );
    for arguments in [&[7, 0, 0][..], &[i64::MIN, -1, 0][..], &[9, 2, 4][..]] {
        assert_eq!(
            machine_eval::evaluate(&baseline, arguments),
            machine_eval::evaluate(&function, arguments)
        );
    }
    for (arguments, expected) in [
        (&[7, 0, 1][..], MachineEvalError::DivisionByZero),
        (&[i64::MIN, -1, 1][..], MachineEvalError::DivisionOverflow),
    ] {
        assert_eq!(
            machine_eval::evaluate(&baseline, arguments),
            Err(expected.clone())
        );
        assert_eq!(machine_eval::evaluate(&function, arguments), Err(expected));
    }
}
