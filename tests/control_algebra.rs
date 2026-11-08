use std::hint::black_box;
use std::time::{Duration, Instant};

use kimi_fast_rustc::son::{
    BinaryOp, CompareOp, FunctionMetadata, Graph, NodeId, NodeKind, UnaryOp,
};
use kimi_fast_rustc::x86::MachineFunction;
use kimi_fast_rustc::{
    frontend, machine_eval, native, native_program, optimizer_pipeline, placement, son_optimize,
};

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

fn reference_constant_fold(graph: &mut Graph) -> usize {
    let ids: Vec<_> = graph.ids().collect();
    let mut folded = 0;
    for id in ids {
        let Some(value) = son_optimize::constant_value(graph, id) else {
            continue;
        };
        let replacement = graph.constant(value);
        graph.replace_uses(id, replacement);
        folded += 1;
    }
    folded
}

fn reference_optimize(graph: &mut Graph) -> son_optimize::OptimizeStats {
    let mut stats = son_optimize::OptimizeStats::default();
    loop {
        let folded = reference_constant_fold(graph);
        let simplified = son_optimize::algebraic_simplify(graph);
        stats.constants_folded += folded;
        stats.algebraic_simplifications += simplified;
        stats.dead_nodes_removed += son_optimize::eliminate_dead_nodes(graph);
        if folded == 0 && simplified == 0 {
            break;
        }
    }
    stats
}

fn assert_same_graph(actual: &Graph, expected: &Graph) {
    assert_eq!(actual.len(), expected.len());
    assert_eq!(actual.functions, expected.functions);
    assert_eq!(
        actual.iter().collect::<Vec<_>>(),
        expected.iter().collect::<Vec<_>>()
    );
}

fn deep_constant_graph(chains: usize, depth: usize) -> (Graph, Vec<NodeId>) {
    let mut graph = Graph::new();
    let start = graph.start();
    let mut expressions = Vec::with_capacity(chains);
    for chain in 0..chains {
        let initial = graph.constant(chain as i64);
        let increment = graph.constant(1);
        let mut value = initial;
        for _ in 0..depth {
            value = graph.binary(BinaryOp::Add, value, increment);
        }
        let negated = graph.unary(UnaryOp::Neg, value);
        let restored = graph.unary(UnaryOp::Neg, negated);
        let inverted = graph.unary(UnaryOp::Not, restored);
        let expected = graph.constant(!(chain as i64 + depth as i64));
        let comparison = graph.compare(CompareOp::Eq, inverted, expected);
        graph.return_(start, Some(comparison));
        expressions.push(comparison);
    }
    (graph, expressions)
}

#[test]
fn deep_constant_expressions_match_sequential_replacement_exactly() {
    const DEPTH: usize = 128;
    let (graph, expressions) = deep_constant_graph(1, DEPTH);
    let original_nodes = graph.len();
    let mut expected = graph.clone();
    let expected_folded = reference_constant_fold(&mut expected);
    let mut actual = graph;
    let actual_folded = son_optimize::constant_fold(&mut actual);

    assert_eq!(actual_folded, DEPTH + 4);
    assert_eq!(actual_folded, expected_folded);
    assert_eq!(actual.len(), original_nodes + actual_folded);
    assert_same_graph(&actual, &expected);
    for expression in expressions {
        assert_eq!(son_optimize::constant_value(&actual, expression), Some(1));
        assert_eq!(
            son_optimize::constant_value(&actual, expression),
            son_optimize::constant_value(&expected, expression)
        );
    }
    actual.validate().unwrap();
}

#[test]
fn multi_function_external_constants_match_sequential_optimization() {
    let mut graph = Graph::new();
    let shared_left = graph.constant(20);
    let shared_right = graph.constant(22);
    let shared_sum = graph.binary(BinaryOp::Add, shared_left, shared_right);

    let first_start = graph.start();
    let multiplier = graph.constant(1);
    let product = graph.binary(BinaryOp::Mul, shared_sum, multiplier);
    let negated = graph.unary(UnaryOp::Neg, product);
    let first_result = graph.unary(UnaryOp::Neg, negated);
    let first_return = graph.return_(first_start, Some(first_result));
    graph.functions.push(FunctionMetadata {
        name: "first".into(),
        start: first_start,
        return_node: first_return,
        parameter_count: 0,
    });

    let main_start = graph.start();
    let subtrahend = graph.constant(2);
    let difference = graph.binary(BinaryOp::Sub, shared_sum, subtrahend);
    let increment = graph.constant(1);
    let main_result = graph.binary(BinaryOp::Add, difference, increment);
    let main_return = graph.return_(main_start, Some(main_result));
    graph.functions.push(FunctionMetadata {
        name: "main".into(),
        start: main_start,
        return_node: main_return,
        parameter_count: 0,
    });

    let mut expected = graph.clone();
    let expected_stats = reference_optimize(&mut expected);
    let mut actual = graph;
    let actual_stats = son_optimize::optimize(&mut actual);

    assert_eq!(actual_stats.constants_folded, 6);
    assert_eq!(actual_stats, expected_stats);
    assert_same_graph(&actual, &expected);
    for index in 0..actual.functions.len() {
        assert_same_graph(
            &actual.function_graph(&actual.functions[index]),
            &expected.function_graph(&expected.functions[index]),
        );
    }

    let actual_program = native_program::compile_graph(&actual).unwrap();
    let expected_program = native_program::compile_graph(&expected).unwrap();
    assert_eq!(actual_program.functions, expected_program.functions);
    for (name, result) in [("first", 42), ("main", 41)] {
        let function = actual_program
            .functions
            .iter()
            .find(|function| function.name == name)
            .unwrap();
        assert_eq!(machine_eval::evaluate(function, &[]), Ok(result));
    }
    actual.validate().unwrap();
}

fn time_constant_fold(graph: &Graph, fold: fn(&mut Graph) -> usize) -> (Duration, usize, Graph) {
    let mut graph = graph.clone();
    let started = Instant::now();
    let folded = fold(black_box(&mut graph));
    let elapsed = started.elapsed();
    (elapsed, black_box(folded), graph)
}

fn median(mut samples: Vec<Duration>) -> Duration {
    samples.sort_unstable();
    samples[samples.len() / 2]
}

#[test]
#[ignore = "timing comparison; run with --release -- --ignored --nocapture"]
fn repeatable_constant_fold_timing_comparison() {
    const CHAINS: usize = 64;
    const DEPTH: usize = 64;
    const ROUNDS: usize = 9;
    let (graph, _) = deep_constant_graph(CHAINS, DEPTH);
    let mut expected_folded = None;
    let mut reference_samples = Vec::with_capacity(ROUNDS);
    let mut optimized_samples = Vec::with_capacity(ROUNDS);

    for round in 0..ROUNDS + 2 {
        let (reference, optimized) = if round % 2 == 0 {
            (
                time_constant_fold(&graph, reference_constant_fold),
                time_constant_fold(&graph, son_optimize::constant_fold),
            )
        } else {
            let optimized = time_constant_fold(&graph, son_optimize::constant_fold);
            let reference = time_constant_fold(&graph, reference_constant_fold);
            (reference, optimized)
        };
        assert_eq!(optimized.1, reference.1);
        assert_eq!(*expected_folded.get_or_insert(reference.1), reference.1);
        assert_same_graph(&optimized.2, &reference.2);
        if round >= 2 {
            println!(
                "round {}: reference={:?} optimized={:?}",
                round - 1,
                reference.0,
                optimized.0
            );
            reference_samples.push(reference.0);
            optimized_samples.push(optimized.0);
        }
    }

    let reference = median(reference_samples);
    let optimized = median(optimized_samples);
    println!(
        "median over {ROUNDS} rounds: nodes={} folds={} reference={reference:?} \
         optimized={optimized:?} speedup={:.2}x",
        graph.len(),
        expected_folded.unwrap(),
        reference.as_secs_f64() / optimized.as_secs_f64()
    );
}
