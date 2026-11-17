//! Incoming predicates may be known even when the tested value is not a Phi.
use super::tests::{call, projection};
use super::*;
use crate::{GraphLevel, InterpretError, Interpreter, verify_graph};
use kfrc_body::{FunctionAbi, IntegerWidth, PassMode, RecordedTypes, SizeAlign};

fn fixture(fresh: bool, reverse: bool) -> FunctionGraph {
    let mut types = RecordedTypes::new();
    let word = types.intern(
        TypeShape::Int {
            width: IntegerWidth::W64,
            signed: false,
        },
        Some(SizeAlign { size: 8, align: 8 }),
    );
    let boolean = types.intern(TypeShape::Bool, Some(SizeAlign { size: 1, align: 1 }));
    let mut graph = FunctionGraph::new(
        "edge_predicate",
        FunctionAbi {
            args: vec![PassMode::Direct],
            ret: PassMode::Direct,
            c_variadic: false,
        },
        types,
    );
    graph.level = GraphLevel::Layout;
    let start = graph.add_node(
        Opcode::Start,
        NodeId::NONE,
        &[],
        0,
        ValueType::Tuple(vec![
            ValueType::Control,
            ValueType::Effect,
            ValueType::Rust(word),
        ]),
    );
    graph.start = start;
    let control = projection(&mut graph, start, 0, ValueType::Control);
    let effect = projection(&mut graph, start, 1, ValueType::Effect);
    let value = projection(&mut graph, start, 2, ValueType::Rust(word));
    let zero = graph.add_constant(0, word);
    let condition = graph.add_node(
        Opcode::Compare,
        NodeId::NONE,
        &[value, zero],
        ComparePredicate::Equal as u32,
        ValueType::Rust(boolean),
    );
    let branch = graph.add_node(
        Opcode::If,
        control,
        &[condition],
        0,
        ValueType::Tuple(vec![ValueType::Control; 2]),
    );
    let mut edges = Vec::new();
    let mut effects = Vec::new();
    let mut values = Vec::new();
    for (side, name) in ["left", "right"].into_iter().enumerate() {
        let control = projection(&mut graph, branch, side as u32, ValueType::Control);
        let (control, effect, value) = call(&mut graph, control, effect, value, word, name);
        edges.push(graph.add_node(Opcode::Jump, control, &[], 0, ValueType::Control));
        effects.push(effect);
        values.push(value);
    }
    if reverse {
        edges.reverse();
        effects.reverse();
        values.reverse();
    }
    let region = graph.add_node(Opcode::Region, NodeId::NONE, &edges, 0, ValueType::Control);
    let effect = graph.add_node(
        Opcode::Phi,
        NodeId::NONE,
        &[region, effects[0], effects[1]],
        0,
        ValueType::Effect,
    );
    let merged = graph.add_node(
        Opcode::Phi,
        NodeId::NONE,
        &[region, values[0], values[1]],
        0,
        ValueType::Rust(word),
    );
    let condition = graph.add_node(
        Opcode::Compare,
        NodeId::NONE,
        &[if fresh { merged } else { value }, zero],
        ComparePredicate::NotEqual as u32,
        ValueType::Rust(boolean),
    );
    let branch = graph.add_node(
        Opcode::If,
        region,
        &[condition],
        0,
        ValueType::Tuple(vec![ValueType::Control; 2]),
    );
    let mut ends = Vec::new();
    let mut results = Vec::new();
    for side in 0..2 {
        let control = projection(&mut graph, branch, side, ValueType::Control);
        let add = graph.add_constant(100 * u128::from(side + 1), word);
        results.push(graph.add_node(
            Opcode::Add,
            NodeId::NONE,
            &[merged, add],
            0,
            ValueType::Rust(word),
        ));
        ends.push(graph.add_node(Opcode::Jump, control, &[], 0, ValueType::Control));
    }
    let end = graph.add_node(Opcode::Region, NodeId::NONE, &ends, 0, ValueType::Control);
    let result = graph.add_node(
        Opcode::Phi,
        NodeId::NONE,
        &[end, results[0], results[1]],
        0,
        ValueType::Rust(word),
    );
    graph.add_node(
        Opcode::Return,
        end,
        &[effect, result],
        0,
        ValueType::Control,
    );
    graph
}

#[test]
fn incoming_observation_routes_edges_and_repairs_values_after_fixed_calls() {
    for fresh in [false, true] {
        for reverse in [false, true] {
            let mut graph = fixture(fresh, reverse);
            verify_graph(&mut graph).unwrap();
            let original = graph.clone();
            let stats = run_with_edge_facts(&mut graph, false, true).unwrap();
            assert_eq!(
                stats.regions_threaded,
                usize::from(!fresh),
                "{stats:?}\n{}",
                graph.dump()
            );
            verify_graph(&mut graph).unwrap_or_else(|error| panic!("{error}\n{}", graph.dump()));
            for input in [0, 1, 8, u64::MAX] {
                for returned in [0, 1, u64::MAX] {
                    for fail in [false, true] {
                        let execute = |graph: &FunctionGraph| {
                            let mut calls = Vec::new();
                            let outcome = Interpreter::new(graph).run(
                                &[input],
                                &mut |name: &str, args: &[u64]| {
                                    calls.push((name.to_owned(), args.to_vec()));
                                    if fail {
                                        Err(InterpretError::UnknownCallee(name.into()))
                                    } else {
                                        Ok(returned)
                                    }
                                },
                            );
                            (outcome, calls)
                        };
                        assert_eq!(
                            execute(&original),
                            execute(&graph),
                            "fresh {fresh}, reverse {reverse}, input {input}, returned {returned}, fail {fail}"
                        );
                    }
                }
            }
        }
    }
}
