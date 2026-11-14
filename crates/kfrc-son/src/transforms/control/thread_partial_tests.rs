use super::*;
use crate::{GraphLevel, Interpreter};
use kfrc_body::{FunctionAbi, IntegerWidth, PassMode, RecordedTypes};

use super::tests::{call, execute, projection};

fn fixture(
    level: GraphLevel,
    poison: bool,
    reconverge: bool,
    with_loop: bool,
) -> (FunctionGraph, NodeId) {
    let mut types = RecordedTypes::new();
    let word = types.intern(
        TypeShape::Int {
            width: IntegerWidth::W64,
            signed: false,
        },
        None,
    );
    let boolean = types.intern(TypeShape::Bool, None);
    let mut graph = FunctionGraph::new(
        "partial_predecessors",
        FunctionAbi {
            args: vec![PassMode::Direct],
            ret: PassMode::Direct,
            c_variadic: false,
        },
        types,
    );
    graph.level = level;
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
    let argument = projection(&mut graph, start, 2, ValueType::Rust(word));
    let zero = graph.add_constant(0, word);
    let one = graph.add_constant(1, word);
    let mut control = control;
    let mut entries = Vec::new();
    for constant in [zero, one] {
        let condition = graph.add_node(
            Opcode::Compare,
            NodeId::NONE,
            &[argument, constant],
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
        entries.push(projection(&mut graph, branch, 0, ValueType::Control));
        control = projection(&mut graph, branch, 1, ValueType::Control);
    }
    entries.push(control);
    let mut edges = Vec::new();
    let mut effects = Vec::new();
    let mut payloads = Vec::new();
    for (control, name) in entries
        .into_iter()
        .zip(["known-true", "known-false", "dynamic"])
    {
        let (control, after, value) = call(&mut graph, control, effect, argument, word, name);
        edges.push(graph.add_node(Opcode::Jump, control, &[], 0, ValueType::Control));
        effects.push(after);
        payloads.push(value);
    }
    let root = graph.add_node(Opcode::Region, NodeId::NONE, &edges, 0, ValueType::Control);
    let yes = graph.add_constant(1, boolean);
    let no = graph.add_constant(0, boolean);
    let masked = graph.add_node(
        Opcode::BitAnd,
        NodeId::NONE,
        &[argument, one],
        0,
        ValueType::Rust(word),
    );
    let unknown = if poison {
        graph.add_node(
            Opcode::Undefined,
            NodeId::NONE,
            &[],
            0,
            ValueType::Rust(boolean),
        )
    } else {
        graph.add_node(
            Opcode::Compare,
            NodeId::NONE,
            &[masked, zero],
            ComparePredicate::Equal as u32,
            ValueType::Rust(boolean),
        )
    };
    let tag = graph.add_node(
        Opcode::Phi,
        NodeId::NONE,
        &[root, yes, no, unknown],
        0,
        ValueType::Rust(boolean),
    );
    let effect = graph.add_node(
        Opcode::Phi,
        root,
        &[root, effects[0], effects[1], effects[2]],
        0,
        ValueType::Effect,
    );
    let payload = graph.add_node(
        Opcode::Phi,
        NodeId::NONE,
        &[root, payloads[0], payloads[1], payloads[2]],
        0,
        ValueType::Rust(word),
    );
    let branch = graph.add_node(
        Opcode::If,
        root,
        &[tag],
        0,
        ValueType::Tuple(vec![ValueType::Control; 2]),
    );
    let mut joined_edges = Vec::new();
    let mut joined_effects = Vec::new();
    for (side, name) in ["yes", "no"].into_iter().enumerate() {
        let control = projection(&mut graph, branch, side as u32, ValueType::Control);
        let increment = graph.add_constant((side + 3) as u128, word);
        let value = graph.add_node(
            Opcode::Add,
            NodeId::NONE,
            &[payload, increment],
            0,
            ValueType::Rust(word),
        );
        let (control, after, result) = call(&mut graph, control, effect, value, word, name);
        if reconverge {
            joined_edges.push(graph.add_node(Opcode::Jump, control, &[], 0, ValueType::Control));
            joined_effects.push(after);
        } else {
            graph.add_node(
                Opcode::Return,
                control,
                &[after, result],
                0,
                ValueType::Control,
            );
        }
    }
    if reconverge {
        let joined = graph.add_node(
            Opcode::Region,
            NodeId::NONE,
            &joined_edges,
            0,
            ValueType::Control,
        );
        let after = graph.add_node(
            Opcode::Phi,
            joined,
            &[joined, joined_effects[0], joined_effects[1]],
            0,
            ValueType::Effect,
        );
        let (control, after, late) = call(&mut graph, joined, after, argument, word, "late");
        let value = graph.add_node(
            Opcode::Add,
            NodeId::NONE,
            &[payload, late],
            0,
            ValueType::Rust(word),
        );
        let (control, value) = if with_loop {
            let entry = graph.add_node(Opcode::Jump, control, &[], 0, ValueType::Control);
            let latch = graph.add_node(Opcode::Jump, control, &[], 0, ValueType::Control);
            let header = graph.add_node(
                Opcode::Region,
                NodeId::NONE,
                &[entry, latch],
                0,
                ValueType::Control,
            );
            let counter = graph.add_node(
                Opcode::Phi,
                NodeId::NONE,
                &[header, masked, masked],
                0,
                ValueType::Rust(word),
            );
            let carried = graph.add_node(
                Opcode::Phi,
                NodeId::NONE,
                &[header, value, value],
                0,
                ValueType::Rust(word),
            );
            let condition = graph.add_node(
                Opcode::Compare,
                NodeId::NONE,
                &[counter, zero],
                ComparePredicate::NotEqual as u32,
                ValueType::Rust(boolean),
            );
            let check = graph.add_node(
                Opcode::If,
                header,
                &[condition],
                0,
                ValueType::Tuple(vec![ValueType::Control; 2]),
            );
            let again = projection(&mut graph, check, 0, ValueType::Control);
            let exit = projection(&mut graph, check, 1, ValueType::Control);
            let decrement = graph.add_node(
                Opcode::Sub,
                NodeId::NONE,
                &[counter, one],
                0,
                ValueType::Rust(word),
            );
            graph.set_control(latch, again);
            graph.set_input(counter, 2, decrement);
            (exit, carried)
        } else {
            (control, value)
        };
        graph.add_node(
            Opcode::Return,
            control,
            &[after, value],
            0,
            ValueType::Control,
        );
    }
    (graph, branch)
}

#[test]
fn known_predecessors_bypass_a_residual_branch_with_its_own_effects() {
    for level in [GraphLevel::Specialized, GraphLevel::Layout] {
        for (poison, reconverge, with_loop) in [
            (false, false, false),
            (false, true, false),
            (true, true, false),
            (false, true, true),
            (true, true, true),
        ] {
            let (mut graph, branch) = fixture(level, poison, reconverge, with_loop);
            crate::verify_graph(&mut graph).unwrap();
            let observe = |graph: &FunctionGraph, input| {
                if level == GraphLevel::Specialized {
                    let (outcome, trace) = execute(graph, input);
                    (format!("{outcome:?}"), trace)
                } else {
                    let mut trace = Vec::new();
                    let outcome =
                        Interpreter::new(graph).run(&[input], &mut |name: &str, args: &[u64]| {
                            trace.push(format!("{name}:{args:?}"));
                            Ok(args[0])
                        });
                    (format!("{outcome:?}"), trace)
                }
            };
            let inputs = [0, 1, 2, 3, 4, 5, u64::MAX];
            let expected = inputs.map(|input| observe(&graph, input));
            let stats = super::run(&mut graph).unwrap();
            assert_eq!(stats.predecessors_threaded, 2, "{}", graph.dump());
            assert_eq!(stats.branches_removed, 0);
            assert!(!graph.is_dead(branch));
            crate::verify_graph(&mut graph).unwrap();
            for (input, expected) in inputs.into_iter().zip(expected) {
                assert_eq!(
                    observe(&graph, input),
                    expected,
                    "level={level:?}, poison={poison}, reconverge={reconverge}, input={input}"
                );
            }
        }
    }
}

#[test]
fn wholly_dynamic_predecessors_do_not_change_the_graph() {
    let (mut graph, branch) = fixture(GraphLevel::Specialized, false, true, false);
    let tag = graph.input(branch, 0);
    let dynamic = graph.input(tag, 3);
    graph.set_input(tag, 1, dynamic);
    graph.set_input(tag, 2, dynamic);
    crate::verify_graph(&mut graph).unwrap();
    let before = graph.dump();
    let stats = super::run(&mut graph).unwrap();
    assert_eq!(stats.predecessors_threaded, 0);
    assert_eq!(stats.branches_removed, 0);
    assert_eq!(graph.dump(), before);
}
