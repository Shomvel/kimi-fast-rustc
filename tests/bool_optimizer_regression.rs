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
