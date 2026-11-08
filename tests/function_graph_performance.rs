use std::collections::{BTreeMap, HashSet};
use std::hint::black_box;
use std::time::{Duration, Instant};

use kimi_fast_rustc::native_program::NativeProgram;
use kimi_fast_rustc::son::{BinaryOp, FunctionMetadata, Graph, InputChannel, NodeId, NodeKind};
use kimi_fast_rustc::{frontend, native, native_program, optimizer_pipeline};

fn graph_with_functions_and_constants(function_count: usize, constant_count: usize) -> Graph {
    assert!(constant_count >= 3);
    let mut source = String::new();
    for index in 0..constant_count {
        source.push_str(&format!("const C{index}: i64 = {};\n", index * 2 + 3));
    }
    for index in 0..function_count {
        let initial = index % constant_count;
        let addend = (index + 1) % constant_count;
        source.push_str(&format!(
            "fn function_{index}(value: i64) -> i64 {{ \
             let mut counter = 0; let mut total = C{initial}; \
             while counter < 2 {{ total = total + value * 16; counter = counter + 1; }} \
             return total + C{addend}; }}\n"
        ));
    }
    frontend::parse(&source).unwrap()
}

fn reference_function_graph(graph: &Graph, function: &FunctionMetadata) -> Graph {
    let in_range: HashSet<NodeId> = graph
        .ids()
        .filter(|id| id.0 >= function.start.0 && id.0 <= function.return_node.0)
        .collect();
    let mut retained = in_range.clone();
    let mut stack = retained.iter().copied().collect::<Vec<_>>();
    while let Some(id) = stack.pop() {
        let node = graph.node(id).expect("retained node exists");
        for channel in [
            InputChannel::Control,
            InputChannel::Data,
            InputChannel::Memory,
        ] {
            for &input in node.inputs(channel) {
                if graph.node(input).is_some() && retained.insert(input) {
                    stack.push(input);
                }
            }
        }
    }

    let mut ordered = vec![function.start];
    ordered.extend(
        graph
            .ids()
            .filter(|id| retained.contains(id) && !in_range.contains(id)),
    );
    ordered.extend(
        graph
            .ids()
            .filter(|id| in_range.contains(id) && *id != function.start),
    );
    let mapping: BTreeMap<NodeId, NodeId> = ordered
        .iter()
        .enumerate()
        .map(|(index, id)| (*id, NodeId(index as u32)))
        .collect();
    let mut extracted = Graph::new();
    for old_id in ordered {
        let mut node = graph.node(old_id).expect("ordered node exists").clone();
        for channel in [
            InputChannel::Control,
            InputChannel::Data,
            InputChannel::Memory,
        ] {
            for input in node.inputs_mut(channel) {
                if let Some(mapped) = mapping.get(input) {
                    *input = *mapped;
                }
            }
        }
        extracted.push(node);
    }
    extracted.functions = graph.functions.clone();
    for metadata in &mut extracted.functions {
        if let Some(mapped) = mapping.get(&metadata.start) {
            metadata.start = *mapped;
        }
        if let Some(mapped) = mapping.get(&metadata.return_node) {
            metadata.return_node = *mapped;
        }
    }
    extracted
}

fn assert_same_graph(actual: &Graph, expected: &Graph) {
    assert_eq!(actual.len(), expected.len());
    assert_eq!(actual.functions, expected.functions);
    assert_eq!(
        actual.iter().collect::<Vec<_>>(),
        expected.iter().collect::<Vec<_>>()
    );
}

#[test]
fn holes_and_transitive_external_dependencies_match_the_reference() {
    let mut graph = Graph::new();
    let root = graph.constant(40);
    let addend = graph.constant(2);
    let removed_external = graph.constant(999);
    let external = graph.binary(BinaryOp::Add, root, addend);
    let unused = graph.constant(777);
    graph.remove(removed_external);

    let start = graph.start();
    let parameter = graph.parameter(0);
    let removed_local = graph.constant(123);
    graph.remove(removed_local);
    let sum = graph.binary(BinaryOp::Add, parameter, external);
    let call = graph.add_node_with_memory(NodeKind::Call(0), vec![start], vec![sum], vec![root]);
    let return_node = graph.return_(start, Some(call));
    graph.functions.push(FunctionMetadata {
        name: "main".into(),
        start,
        return_node,
        parameter_count: 1,
    });

    let other_start = graph.start();
    let other_value = graph.constant(9);
    let other_return = graph.return_(other_start, Some(other_value));
    graph.functions.push(FunctionMetadata {
        name: "other".into(),
        start: other_start,
        return_node: other_return,
        parameter_count: 0,
    });

    let expected = reference_function_graph(&graph, &graph.functions[0]);
    let actual = graph.function_graph(&graph.functions[0]);
    assert_same_graph(&actual, &expected);
    actual.validate().unwrap();
    assert_eq!(actual.len(), 8);
    assert_eq!(actual.functions[0].start, NodeId(0));
    assert_eq!(actual.functions[0].return_node, NodeId(7));
    assert_eq!(actual.functions[1].start, other_start);
    assert_eq!(actual.functions[1].return_node, other_return);
    assert_eq!(
        actual.node(NodeId(5)).unwrap().data,
        vec![NodeId(4), NodeId(3)]
    );
    assert_eq!(actual.node(NodeId(6)).unwrap().control, vec![NodeId(0)]);
    assert_eq!(actual.node(NodeId(6)).unwrap().memory, vec![NodeId(1)]);
    assert_eq!(graph.node(unused).unwrap().kind, NodeKind::Const(777));
    assert!(
        actual
            .iter()
            .all(|(_, node)| node.kind != NodeKind::Const(777))
    );
}

#[test]
fn extraction_optimization_and_elf_match_the_reference() {
    let graph = graph_with_functions_and_constants(8, 16);
    let mut expected_functions = Vec::new();
    let mut actual_functions = Vec::new();

    for metadata in &graph.functions {
        let expected = reference_function_graph(&graph, metadata);
        let actual = graph.function_graph(metadata);
        assert_same_graph(&actual, &expected);

        expected_functions.push(native::compile_graph(&expected, metadata.name.clone()).unwrap());
        actual_functions.push(native::compile_graph(&actual, metadata.name.clone()).unwrap());

        let original_len = actual.len();
        let mut expected = expected;
        let mut actual = actual;
        let expected_stats = optimizer_pipeline::optimize(&mut expected);
        let actual_stats = optimizer_pipeline::optimize(&mut actual);
        assert_eq!(actual_stats, expected_stats);
        assert_same_graph(&actual, &expected);
        assert_eq!(
            actual.node(NodeId(original_len as u32)).unwrap().kind,
            kimi_fast_rustc::son::NodeKind::Const(4),
            "strength reduction must retain the same appended constant ID"
        );
    }

    assert_eq!(actual_functions, expected_functions);
    let actual = NativeProgram {
        functions: actual_functions,
    }
    .emit_object()
    .unwrap();
    let expected = NativeProgram {
        functions: expected_functions,
    }
    .emit_object()
    .unwrap();
    assert_eq!(actual, expected);
    assert_eq!(&actual[..4], b"\x7fELF");

    let direct = native_program::compile_graph_to_object(&graph).unwrap();
    let reference = NativeProgram {
        functions: graph
            .functions
            .iter()
            .map(|metadata| {
                native::compile_graph(
                    &reference_function_graph(&graph, metadata),
                    metadata.name.clone(),
                )
                .unwrap()
            })
            .collect(),
    }
    .emit_object()
    .unwrap();
    assert_eq!(direct, reference);
}

fn time_extraction(
    graph: &Graph,
    extract: impl Fn(&Graph, &FunctionMetadata) -> Graph,
) -> (Duration, usize) {
    let started = Instant::now();
    let mut extracted_nodes = 0;
    for metadata in &graph.functions {
        extracted_nodes += black_box(extract(graph, metadata)).len();
    }
    (started.elapsed(), black_box(extracted_nodes))
}

fn median(mut samples: Vec<Duration>) -> Duration {
    samples.sort_unstable();
    samples[samples.len() / 2]
}

#[test]
#[ignore = "repeatable timing comparison; run with --ignored --nocapture"]
fn repeatable_function_graph_timing_comparison() {
    const ROUNDS: usize = 9;
    let graph = graph_with_functions_and_constants(96, 64);
    let original_nodes = graph.len();
    let original_functions = graph.functions.len();
    let mut expected_count = None;
    let mut reference_samples = Vec::with_capacity(ROUNDS);
    let mut optimized_samples = Vec::with_capacity(ROUNDS);

    for round in 0..ROUNDS + 2 {
        let reference = || time_extraction(&graph, reference_function_graph);
        let optimized = || time_extraction(&graph, Graph::function_graph);
        let (reference_time, optimized_time, count) = if round % 2 == 0 {
            let (reference_time, count) = reference();
            let (optimized_time, optimized_count) = optimized();
            assert_eq!(optimized_count, count);
            (reference_time, optimized_time, count)
        } else {
            let (optimized_time, count) = optimized();
            let (reference_time, reference_count) = reference();
            assert_eq!(reference_count, count);
            (reference_time, optimized_time, count)
        };
        assert_eq!(*expected_count.get_or_insert(count), count);
        if round >= 2 {
            println!(
                "round {}: reference={reference_time:?} optimized={optimized_time:?}",
                round - 1
            );
            reference_samples.push(reference_time);
            optimized_samples.push(optimized_time);
        }
    }

    let reference = median(reference_samples);
    let optimized = median(optimized_samples);
    println!(
        "median over {ROUNDS} rounds: functions={original_functions} nodes={original_nodes} \
         reference={reference:?} optimized={optimized:?} speedup={:.2}x",
        reference.as_secs_f64() / optimized.as_secs_f64()
    );
}
