use super::*;
use crate::{Opcode, ValueType};
use kfrc_body::{FunctionAbi, IntegerWidth, PassMode, RecordedTypes, TyId, TypeShape};

fn graph() -> (FunctionGraph, TyId) {
    let mut types = RecordedTypes::new();
    let word = types.intern(
        TypeShape::Int {
            signed: false,
            width: IntegerWidth::W64,
        },
        None,
    );
    let mut graph = FunctionGraph::new(
        "use_snapshot",
        FunctionAbi {
            args: vec![],
            ret: PassMode::Direct,
            c_variadic: false,
        },
        types,
    );
    graph.start = graph.add_node(
        Opcode::Start,
        NodeId::NONE,
        &[],
        0,
        ValueType::Tuple(vec![ValueType::Control, ValueType::Effect]),
    );
    (graph, word)
}

// A separate, direct edge walk checks contents and ordering without consulting
// the cached builder, counting-sort representation or forwarding storage.
fn check(graph: &FunctionGraph) -> Uses {
    let actual = Uses::new(graph);
    let mut expected = vec![Vec::new(); graph.len()];
    for user in graph.ids() {
        for (slot, &input) in graph.inputs(user).iter().enumerate() {
            expected[graph.resolve(input).index()].push((user, UseKind::Operand(slot)));
        }
        let control = graph.resolve(graph.control(user));
        if !control.is_none() {
            expected[control.index()].push((user, UseKind::Control));
        }
    }
    for (node, uses) in expected.iter().enumerate() {
        let entries: Vec<_> = actual
            .of(NodeId(node as u32))
            .iter()
            .map(|usage| (usage.user, usage.kind()))
            .collect();
        assert_eq!(&entries, uses, "node {node}");
    }
    assert!(!actual.contains(NodeId::NONE));
    assert!(!actual.contains(NodeId(graph.len() as u32)));
    actual
}

#[test]
fn snapshots_survive_forwarding_growth_retirement_and_compaction() {
    let (mut graph, word) = graph();
    let a = graph.add_constant(10, word);
    let b = graph.add_constant(20, word);
    let sum = graph.add_node(Opcode::Add, NodeId::NONE, &[a, a], 0, ValueType::Rust(word));
    let initial = check(&graph);
    assert!(Arc::ptr_eq(&initial.index, &check(&graph).index));
    let original_a = initial.of(a).to_vec();
    graph.set_input(sum, 0, b);
    let edited = check(&graph);
    assert!(!Arc::ptr_eq(&initial.index, &edited.index));
    assert_eq!(initial.of(a), original_a);
    graph.replace(a, b);
    let forwarded = check(&graph);
    assert_eq!(forwarded.of(b).len(), 2);
    assert!(!Arc::ptr_eq(&edited.index, &forwarded.index));
    graph.resolve_edges();
    check(&graph);
    let extra = graph.add_growable_node(Opcode::Phi, NodeId::NONE, 3, 0, ValueType::Rust(word));
    graph.set_inputs(extra, &[graph.start, b, b]);
    let grown = check(&graph);
    assert!(!initial.contains(extra));
    assert!(grown.contains(extra));
    graph.set_control(sum, graph.start);
    check(&graph);
    graph.remove_node(extra);
    check(&graph);
    graph.add_node(Opcode::Return, graph.start, &[sum], 0, ValueType::Control);
    let before_compact = check(&graph);
    graph.compact();
    let compacted = check(&graph);
    assert!(!Arc::ptr_eq(&before_compact.index, &compacted.index));
    assert_eq!(initial.of(a), original_a);
}

#[test]
fn cloned_and_concurrent_readers_share_only_unchanged_snapshots() {
    let (graph, word) = graph();
    let initial = check(&graph);
    let mut fork = graph.clone();
    assert!(Arc::ptr_eq(&initial.index, &check(&fork).index));
    fork.add_constant(3, word);
    assert!(!Arc::ptr_eq(&initial.index, &check(&fork).index));
    assert!(Arc::ptr_eq(&initial.index, &check(&graph).index));
    std::thread::scope(|scope| {
        let readers: Vec<_> = (0..8).map(|_| scope.spawn(|| check(&graph))).collect();
        for reader in readers {
            assert!(Arc::ptr_eq(&initial.index, &reader.join().unwrap().index));
        }
    });
}

#[test]
fn generated_edge_edits_match_the_direct_walk() {
    let (mut graph, word) = graph();
    let values: Vec<_> = (0..8).map(|n| graph.add_constant(n, word)).collect();
    let mut users = Vec::new();
    let mut random = 0x5924_79ab_2379_8301u64;
    for step in 0..256 {
        random = random.wrapping_mul(6364136223846793005).wrapping_add(1);
        let a = values[(random >> 32) as usize % values.len()];
        let b = values[(random >> 40) as usize % values.len()];
        let prior = check(&graph);
        match step % 8 {
            0 => users.push(graph.add_node(
                Opcode::Add,
                NodeId::NONE,
                &[a, b],
                0,
                ValueType::Rust(word),
            )),
            1 => graph.set_input(*users.last().unwrap(), 1, b),
            2 => graph.set_inputs(*users.last().unwrap(), &[b, a]),
            3 => graph.set_control(*users.last().unwrap(), graph.start),
            4 => graph.set_opcode(*users.last().unwrap(), Opcode::Sub),
            5 => graph.set_payload(*users.last().unwrap(), 0),
            6 => graph.set_value_type(*users.last().unwrap(), ValueType::Rust(word)),
            _ => graph.remove_node(*users.last().unwrap()),
        }
        let next = check(&graph);
        // Opcode, payload and type edits leave the actual use edges intact.
        assert_eq!(
            Arc::ptr_eq(&prior.index, &next.index),
            matches!(step % 8, 4..=6)
        );
    }
}

#[test]
fn resolving_forwarded_edges_keeps_the_already_resolved_snapshot() {
    let (mut graph, word) = graph();
    let a = graph.add_constant(10, word);
    let b = graph.add_constant(20, word);
    let sum = graph.add_node(Opcode::Add, NodeId::NONE, &[a, a], 0, ValueType::Rust(word));
    graph.replace(a, b);
    let forwarded = check(&graph);
    graph.resolve_edges();
    assert!(Arc::ptr_eq(&forwarded.index, &check(&graph).index));
    assert_eq!(graph.inputs(sum), &[b, b]);
}

#[test]
fn editing_a_resolved_graph_can_reintroduce_old_aliases() {
    for edit in 0..4 {
        let (mut graph, word) = graph();
        let old = graph.add_constant(10, word);
        let value = graph.add_constant(20, word);
        let user = graph.add_node(
            Opcode::Add,
            NodeId::NONE,
            &[old, value],
            0,
            ValueType::Rust(word),
        );
        graph.replace(old, value);
        graph.resolve_edges();
        assert!(check(&graph).of(old).is_empty());
        match edit {
            0 => graph.set_input(user, 0, old),
            1 => graph.set_inputs(user, &[old, value, old]),
            2 => graph.set_control(user, old),
            _ => {
                graph.add_node(
                    Opcode::Negate,
                    NodeId::NONE,
                    &[old],
                    0,
                    ValueType::Rust(word),
                );
            }
        }
        let dirty = check(&graph);
        assert!(dirty.of(old).is_empty());
        graph.resolve_edges();
        assert!(Arc::ptr_eq(&dirty.index, &check(&graph).index));
    }
}

#[test]
fn pooled_operand_slots_above_u16_stay_distinct_from_control() {
    let (mut graph, word) = graph();
    let value = graph.add_constant(7, word);
    let inputs = vec![value; 65_538];
    let user = graph.add_node(Opcode::Aggregate, value, &inputs, 0, ValueType::Rust(word));
    let uses = check(&graph);
    let entries = uses.of(value);
    assert_eq!(entries.len(), inputs.len() + 1);
    assert_eq!(entries[65_536].user, user);
    assert_eq!(entries[65_536].kind(), UseKind::Operand(65_536));
    assert_eq!(entries.last().unwrap().kind(), UseKind::Control);
}
