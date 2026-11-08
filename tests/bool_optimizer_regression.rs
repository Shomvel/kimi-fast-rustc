use kimi_fast_rustc::son::{BinaryOp, CompareOp, Graph, NodeId, UnaryOp};
use kimi_fast_rustc::x86::MachineFunction;
use kimi_fast_rustc::{
    frontend, machine_eval, native, optimizer_pipeline, placement, value_numbering,
};

fn compile_optimized(source: &str) -> MachineFunction {
    let mut graph = frontend::parse(source).unwrap();
    optimizer_pipeline::optimize(&mut graph);
    graph.validate().unwrap();
    let graph = placement::place_for_native(&graph).unwrap();
    native::compile_graph(&graph, "main").unwrap()
}

#[test]
fn native_bool_compare_example_uses_the_exit_comparison() {
    let mut graph = frontend::parse(include_str!("../examples/native_bool_compare.kfr")).unwrap();
    let body_comparison = NodeId(13);
    let exit_comparison = NodeId(31);
    let exit_branch = NodeId(34);
    optimizer_pipeline::optimize(&mut graph);

    assert_eq!(graph.node(exit_branch).unwrap().data, vec![exit_comparison]);
    assert_ne!(graph.node(exit_branch).unwrap().data, vec![body_comparison]);
    graph.validate().unwrap();
    let graph = placement::place_for_native(&graph).unwrap();
    let function = native::compile_graph(&graph, "main").unwrap();
    assert_eq!(machine_eval::evaluate(&function, &[]), Ok(42));
}

#[derive(Clone, Copy)]
enum Expression {
    Not,
    Add,
    Equal,
    Divide,
}

fn add_expression(
    graph: &mut Graph,
    expression: Expression,
    left: NodeId,
    right: NodeId,
) -> NodeId {
    match expression {
        Expression::Not => graph.unary(UnaryOp::Not, left),
        Expression::Add => graph.binary(BinaryOp::Add, left, right),
        Expression::Equal => graph.compare(CompareOp::Eq, left, right),
        Expression::Divide => graph.binary(BinaryOp::Div, left, right),
    }
}

#[test]
fn non_dominating_phi_expressions_only_merge_within_their_block() {
    for expression in [
        Expression::Not,
        Expression::Add,
        Expression::Equal,
        Expression::Divide,
    ] {
        let mut graph = Graph::new();
        let start = graph.start();
        let parameter = graph.parameter(0);
        let header = graph.region(vec![start]);
        let phi = graph.phi(vec![start], vec![parameter]);
        let branch = graph.if_(header, parameter);
        graph.projection(branch, 0);
        let zero = graph.constant(0);
        let inside = add_expression(&mut graph, expression, phi, zero);
        let duplicate_inside = add_expression(&mut graph, expression, phi, zero);
        let exit = graph.projection(branch, 1);
        let outside = add_expression(&mut graph, expression, phi, zero);
        let result = graph.return_(exit, Some(outside));

        assert_eq!(value_numbering::value_numbering(&mut graph), 1);
        assert_ne!(inside, duplicate_inside);
        assert_eq!(graph.node(result).unwrap().data, vec![outside]);
        graph.validate().unwrap();
    }
}

#[test]
fn dominating_expressions_still_merge_across_regions() {
    let mut graph = Graph::new();
    let start = graph.start();
    let parameter = graph.parameter(0);
    let one = graph.constant(1);
    let before = graph.binary(BinaryOp::Add, parameter, one);
    let branch = graph.if_(start, parameter);
    let body = graph.projection(branch, 0);
    let duplicate = graph.binary(BinaryOp::Add, parameter, one);
    let result = graph.return_(body, Some(duplicate));

    assert_eq!(value_numbering::value_numbering(&mut graph), 1);
    assert_eq!(graph.node(result).unwrap().data, vec![before]);
    graph.validate().unwrap();
}

#[test]
fn branch_local_values_do_not_replace_post_merge_expressions() {
    let function = compile_optimized(
        "fn main(value: i64, flag: bool) -> i64 { let branch_value = if flag { value + 1 } else { 0 }; let after = value + 1; return branch_value + after; }",
    );
    assert_eq!(machine_eval::evaluate(&function, &[41, 0]), Ok(42));
    assert_eq!(machine_eval::evaluate(&function, &[41, 1]), Ok(84));
}

#[test]
fn loop_exit_expressions_use_current_phi_values() {
    for (inside, condition, result, expected) in [
        ("value + 5", "if inside == 0 { value = 7; }", "value + 5", 9),
        ("!value", "if inside == 0 { value = 7; }", "!value", -5),
        (
            "value == 4",
            "if inside { value = 7; }",
            "if value == 4 { 42 } else { 0 }",
            42,
        ),
    ] {
        let source = format!(
            "fn main() -> i64 {{ let mut value = 1; let mut index = 0; while index < 3 {{ let inside = {inside}; {condition} value += 1; index += 1; }} return {result}; }}"
        );
        let function = compile_optimized(&source);
        assert_eq!(
            machine_eval::evaluate(&function, &[]),
            Ok(expected),
            "source: {source}"
        );
    }
}

#[test]
fn large_branch_loop_and_unreachable_graph_is_deterministic() {
    const DIAMONDS: usize = 48;

    let mut graph = Graph::new();
    let start = graph.start();
    let parameter = graph.parameter(0);
    let one = graph.constant(1);
    let shared = graph.binary(BinaryOp::Add, parameter, one);
    let mut value = graph.binary(BinaryOp::Mul, parameter, one);
    let mut control = start;
    let mut branch_local = Vec::new();
    let mut shared_returns = Vec::new();

    for _ in 0..DIAMONDS {
        let branch = graph.if_(control, value);
        let left = graph.projection(branch, 0);
        let left_value = graph.binary(BinaryOp::Add, value, one);
        let duplicate_left = graph.binary(BinaryOp::Add, value, one);
        let left_return = graph.return_(left, Some(duplicate_left));
        let right = graph.projection(branch, 1);
        let right_value = graph.binary(BinaryOp::Add, value, one);
        let duplicate_right = graph.binary(BinaryOp::Add, value, one);
        let right_return = graph.return_(right, Some(duplicate_right));
        control = graph.region(vec![left, right]);
        value = graph.phi(vec![left, right], vec![left_value, right_value]);
        let duplicate_shared = graph.binary(BinaryOp::Add, parameter, one);
        shared_returns.push(graph.return_(control, Some(duplicate_shared)));
        branch_local.push((left_value, left_return, right_value, right_return));
    }

    let header = graph.region(vec![control]);
    let loop_value = graph.phi(vec![control], vec![value]);
    let branch = graph.if_(header, loop_value);
    let body = graph.projection(branch, 0);
    let body_value = graph.binary(BinaryOp::Add, loop_value, one);
    let duplicate_body = graph.binary(BinaryOp::Add, loop_value, one);
    let body_return = graph.return_(body, Some(duplicate_body));
    let latch = graph.region(vec![body]);
    graph.node_mut(header).unwrap().control.push(latch);
    graph.node_mut(loop_value).unwrap().control.push(latch);
    graph.node_mut(loop_value).unwrap().data.push(body_value);
    let exit = graph.projection(branch, 1);
    let duplicate_shared = graph.binary(BinaryOp::Add, parameter, one);
    shared_returns.push(graph.return_(exit, Some(duplicate_shared)));
    let exit_value = graph.binary(BinaryOp::Add, loop_value, one);
    let exit_return = graph.return_(exit, Some(exit_value));

    let unreachable = graph.region(vec![NodeId(graph.len() as u32 + 2)]);
    let unreachable_branch = graph.if_(unreachable, parameter);
    let unreachable_body = graph.projection(unreachable_branch, 0);
    let unreachable_value = graph.binary(BinaryOp::Add, parameter, one);
    let duplicate_unreachable = graph.binary(BinaryOp::Add, parameter, one);
    let unreachable_return = graph.return_(unreachable_body, Some(duplicate_unreachable));
    assert_eq!(
        graph.node(unreachable).unwrap().control,
        vec![unreachable_body]
    );

    let other_start = graph.start();
    let other_parameter = graph.parameter(0);
    let other_value = graph.binary(BinaryOp::Add, other_parameter, one);
    let duplicate_other = graph.binary(BinaryOp::Add, other_parameter, one);
    let other_return = graph.return_(other_start, Some(duplicate_other));

    let before: Vec<_> = graph.iter().map(|(id, node)| (id, node.clone())).collect();
    let mut repeated = graph.clone();
    assert_eq!(
        value_numbering::value_numbering(&mut graph),
        3 * DIAMONDS + 4
    );
    assert_eq!(
        value_numbering::value_numbering(&mut repeated),
        3 * DIAMONDS + 4
    );
    let after: Vec<_> = graph.iter().map(|(id, node)| (id, node.clone())).collect();
    let repeated_after: Vec<_> = repeated
        .iter()
        .map(|(id, node)| (id, node.clone()))
        .collect();
    assert_eq!(after, repeated_after);
    assert_eq!(after.len(), before.len());

    for (left, left_return, right, right_return) in branch_local {
        assert_eq!(graph.node(left_return).unwrap().data, vec![left]);
        assert_eq!(graph.node(right_return).unwrap().data, vec![right]);
    }
    for return_id in shared_returns {
        assert_eq!(graph.node(return_id).unwrap().data, vec![shared]);
    }
    assert_eq!(graph.node(body_return).unwrap().data, vec![body_value]);
    assert_eq!(graph.node(exit_return).unwrap().data, vec![exit_value]);
    assert_eq!(
        graph.node(unreachable_return).unwrap().data,
        vec![unreachable_value]
    );
    assert_eq!(graph.node(other_return).unwrap().data, vec![other_value]);
    graph.validate().unwrap();
    repeated.validate().unwrap();
}
