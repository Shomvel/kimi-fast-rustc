use super::*;
use kfrc_body::{FunctionAbi, PassMode, RecordedTypes};
use std::collections::BTreeSet;

fn graph() -> (FunctionGraph, NodeId, Vec<NodeId>) {
    let mut graph = FunctionGraph::new(
        "cfg_snapshots",
        FunctionAbi {
            args: vec![],
            ret: PassMode::Ignore,
            c_variadic: false,
        },
        RecordedTypes::new(),
    );
    graph.start = graph.add_node(Opcode::Start, NodeId::NONE, &[], 0, ValueType::Control);
    let alternate = graph.add_node(Opcode::Start, NodeId::NONE, &[], 0, ValueType::Control);
    let nodes: Vec<_> = (0..12)
        .map(|_| graph.add_node(Opcode::Region, NodeId::NONE, &[], 0, ValueType::Control))
        .collect();
    for (index, &node) in nodes.iter().enumerate() {
        let mut inputs = vec![nodes[(index + nodes.len() - 1) % nodes.len()]];
        if index == 0 {
            inputs.push(graph.start);
        }
        if index == 4 {
            inputs.push(alternate);
        }
        graph.set_inputs(node, &inputs);
    }
    (graph, alternate, nodes)
}

// Build each adjacency list by direct scanning, independently of the indexed
// counting-sort builder and its cached storage.
fn edges(graph: &FunctionGraph) -> Vec<Vec<NodeId>> {
    let mut edges = vec![Vec::new(); graph.len()];
    for source in graph.ids() {
        for node in graph.ids().filter(|&node| is_control_flow(graph, node)) {
            let mut incoming = vec![graph.control(node)];
            match graph.op(node) {
                Opcode::Projection => incoming.push(graph.input(node, 0)),
                Opcode::Region => incoming.extend_from_slice(graph.inputs(node)),
                _ => {}
            }
            for target in incoming {
                if graph.resolve(target) == source {
                    edges[source.index()].push(node);
                }
            }
        }
    }
    edges
}

fn reachable(edges: &[Vec<NodeId>], start: NodeId, excluded: NodeId) -> BTreeSet<NodeId> {
    let mut seen = BTreeSet::new();
    let mut pending = vec![start];
    while let Some(node) = pending.pop() {
        if node == excluded || node.index() >= edges.len() || !seen.insert(node) {
            continue;
        }
        pending.extend_from_slice(&edges[node.index()]);
    }
    seen
}

fn check(graph: &FunctionGraph) -> ControlFlow {
    let cfg = ControlFlow::new(graph);
    let edges = edges(graph);
    let reached = reachable(&edges, graph.start, NodeId::NONE);
    assert_eq!(cfg.order.iter().copied().collect::<BTreeSet<_>>(), reached);
    for node in graph.ids() {
        assert_eq!(cfg.successors(node), edges[node.index()]);
    }
    let dominators = Dominators::new(&cfg);
    for a in graph.ids() {
        let without_a = reachable(&edges, graph.start, a);
        for b in graph.ids() {
            let expected = reached.contains(&a) && reached.contains(&b) && !without_a.contains(&b);
            assert_eq!(
                dominators.dominates(&cfg, a, b),
                expected,
                "{a} dominates {b}"
            );
        }
    }
    cfg
}

#[test]
fn retained_snapshots_and_clones_survive_graph_edits_and_entry_switches() {
    let (mut graph, alternate, nodes) = graph();
    let initial = check(&graph);
    assert!(Arc::ptr_eq(&initial.data, &check(&graph).data));
    let mut clone = graph.clone();
    assert!(Arc::ptr_eq(&initial.data, &check(&clone).data));
    let original_entry = graph.start;
    graph.start = alternate; // No mutation helper: this field is public.
    let switched = check(&graph);
    assert!(!Arc::ptr_eq(&initial.data, &switched.data));
    graph.start = original_entry;
    assert!(Arc::ptr_eq(&initial.data, &check(&graph).data));
    clone.set_inputs(nodes[1], &[clone.start]);
    let edited = check(&clone);
    assert!(!Arc::ptr_eq(&initial.data, &edited.data));
    assert!(Arc::ptr_eq(&initial.data, &check(&graph).data));
    clone.remove_node(nodes[1]);
    check(&clone);
    clone.compact();
    check(&clone);
    assert_eq!(initial.start, original_entry);
    assert!(initial.data.dominators.get().is_some());
}

#[test]
fn generated_cfg_edits_match_independent_reachability_and_dominance() {
    let (mut graph, _, nodes) = graph();
    let mut random = 0x3213_8902_8923_114bu64;
    for step in 0..96 {
        random = random.wrapping_mul(6364136223846793005).wrapping_add(1);
        let prior = check(&graph);
        let index = step % nodes.len();
        let a = nodes[(random >> 16) as usize % nodes.len()];
        let b = nodes[(random >> 32) as usize % nodes.len()];
        graph.set_inputs(nodes[index], &[a, b, graph.start]);
        assert!(!Arc::ptr_eq(&prior.data, &check(&graph).data));
    }
}

#[test]
fn control_types_forwarding_and_retirement_invalidate_reachability() {
    let (mut graph, _, nodes) = graph();
    let projection = graph.add_node(
        Opcode::Projection,
        NodeId::NONE,
        &[graph.start],
        0,
        ValueType::Effect,
    );
    let effect = check(&graph);
    assert!(!effect.contains(projection));
    graph.set_value_type(projection, ValueType::Control);
    let control = check(&graph);
    assert!(control.contains(projection));
    assert!(!Arc::ptr_eq(&effect.data, &control.data));
    graph.set_control(nodes[3], projection);
    check(&graph);
    graph.replace(nodes[0], nodes[1]);
    check(&graph);
    graph.resolve_edges();
    check(&graph);
    graph.set_opcode(projection, Opcode::Undefined);
    let retired = check(&graph);
    assert!(!retired.contains(projection));
    assert!(control.contains(projection));
}

#[test]
fn concurrent_readers_share_one_cfg_and_lazy_dominators() {
    let (graph, _, _) = graph();
    let cfg = ControlFlow::new(&graph);
    assert!(cfg.data.dominators.get().is_none());
    std::thread::scope(|scope| {
        let readers: Vec<_> = (0..8).map(|_| scope.spawn(|| check(&graph))).collect();
        for reader in readers {
            assert!(Arc::ptr_eq(&cfg.data, &reader.join().unwrap().data));
        }
    });
    assert!(cfg.data.dominators.get().is_some());
}

#[test]
fn scalar_edits_preserve_control_snapshots_and_their_dominators() {
    let (mut graph, _, _) = graph();
    let initial = check(&graph);
    let a = graph.add_node(Opcode::Undefined, NodeId::NONE, &[], 0, ValueType::Effect);
    let b = graph.add_node(Opcode::Undefined, NodeId::NONE, &[], 0, ValueType::Effect);
    let sum = graph.add_node(Opcode::Add, NodeId::NONE, &[a, a], 0, ValueType::Effect);
    let phi = graph.add_growable_node(Opcode::Phi, NodeId::NONE, 2, 0, ValueType::Effect);
    for node in [a, b, sum, phi] {
        assert!(initial.successors(node).is_empty());
        assert!(!initial.contains(node));
    }
    assert!(Arc::ptr_eq(&initial.data, &check(&graph).data));
    graph.set_input(sum, 1, b);
    graph.set_inputs(phi, &[a, b]);
    graph.set_opcode_payload(sum, Opcode::Sub, 1);
    graph.set_value_type(sum, ValueType::Control);
    graph.set_control(sum, graph.start);
    assert!(Arc::ptr_eq(&initial.data, &check(&graph).data));
    // Forwarding only scalar uses cannot change a control predecessor.
    graph.replace(a, b);
    let forwarded = check(&graph);
    graph.resolve_edges();
    assert!(Arc::ptr_eq(&initial.data, &forwarded.data));
    assert!(Arc::ptr_eq(&forwarded.data, &check(&graph).data));
    graph.remove_node(sum);
    assert!(Arc::ptr_eq(&initial.data, &check(&graph).data));
    graph.invalidate(); // Unknown external edits retain conservative invalidation.
    assert!(!Arc::ptr_eq(&initial.data, &check(&graph).data));
}

#[test]
fn scalar_sources_of_temporary_control_edges_invalidate_when_replaced() {
    let (mut graph, _, nodes) = graph();
    let a = graph.add_node(Opcode::Undefined, NodeId::NONE, &[], 0, ValueType::Effect);
    let b = graph.add_node(Opcode::Undefined, NodeId::NONE, &[], 0, ValueType::Effect);
    let c = graph.add_node(Opcode::Undefined, NodeId::NONE, &[], 0, ValueType::Effect);
    // The raw graph API permits temporarily unverified edits. A scalar source
    // of control must still be tracked, even though valid IR never needs it.
    graph.set_inputs(nodes[0], &[graph.start, a]);
    let before = check(&graph);
    graph.replace(a, b);
    let forwarded = check(&graph);
    assert!(!Arc::ptr_eq(&before.data, &forwarded.data));
    // `a` is already forwarded, but the raw Region input still names it.
    graph.replace(a, c);
    let retargeted = check(&graph);
    assert!(!Arc::ptr_eq(&forwarded.data, &retargeted.data));
    graph.resolve_edges();
    assert!(Arc::ptr_eq(&retargeted.data, &check(&graph).data));
    graph.remove_node(c);
    check(&graph);
}
