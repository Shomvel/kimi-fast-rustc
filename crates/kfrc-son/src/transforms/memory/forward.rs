//! Forward a typed snapshot from the immediately preceding identical store.
//!
//! Both control and effect must come directly from the same StoreValue. An
//! unknown call, copy, lifetime change or another access therefore stops this
//! rule. No alias claim follows from a reference's mutability or numeric bits.

use crate::analysis::uses::{UseKind, Uses};
use crate::{FunctionGraph, GraphLevel, NodeId, Opcode, ValueType};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ForwardStats {
    pub loads_forwarded: usize,
}

pub fn run(graph: &mut FunctionGraph) -> ForwardStats {
    let mut stats = ForwardStats::default();
    if !matches!(graph.level, GraphLevel::Rust | GraphLevel::Specialized) {
        return stats;
    }
    graph.resolve_edges();
    let uses = Uses::new(graph);
    let loads: Vec<_> = graph
        .ids()
        .filter(|node| graph.op(*node) == Opcode::LoadValue)
        .collect();
    for load in loads {
        let Some((control, effect, value)) = stored_snapshot(graph, load) else {
            continue;
        };
        // A verified graph only uses a tuple result through its projections.
        // Do not partially rewrite an unexpected tuple consumer.
        let consumers = uses.of(load);
        if consumers.iter().any(|usage| {
            usage.kind != UseKind::Operand(0)
                || graph.op(usage.user) != Opcode::Projection
                || graph.payload(usage.user) > 2
        }) {
            continue;
        }
        // No node is added and each load's tuple users are left untouched until
        // that load is visited, so this precomputed use map remains sufficient.
        for projection in consumers.iter().map(|usage| usage.user) {
            let replacement = match graph.payload(projection) {
                0 => control,
                1 => effect,
                2 => value,
                _ => unreachable!("projection payloads checked above"),
            };
            graph.replace(projection, replacement);
        }
        graph.remove_node(load);
        stats.loads_forwarded += 1;
    }
    graph.resolve_edges();
    stats
}

fn stored_snapshot(graph: &FunctionGraph, load: NodeId) -> Option<(NodeId, NodeId, NodeId)> {
    if graph.inputs(load).len() != 2 {
        return None;
    }
    let effect = graph.resolve(graph.input(load, 0));
    let control = graph.resolve(graph.control(load));
    let store = projection_source(graph, effect, 1)?;
    if projection_source(graph, control, 0)? != store
        || graph.op(store) != Opcode::StoreValue
        || graph.inputs(store).len() != 3
    {
        return None;
    }
    if graph.resolve(graph.input(load, 1)) != graph.resolve(graph.input(store, 1)) {
        return None;
    }
    let value = graph.resolve(graph.input(store, 2));
    let ValueType::Tuple(results) = graph.value_type(load) else {
        return None;
    };
    if results.len() != 3
        || results[0] != ValueType::Control
        || results[1] != ValueType::Effect
        || !matches!(results[2], ValueType::Rust(_))
        || graph.value_type(value) != &results[2]
    {
        return None;
    }
    Some((control, effect, value))
}

fn projection_source(graph: &FunctionGraph, node: NodeId, index: u32) -> Option<NodeId> {
    if node.is_none() || graph.op(node) != Opcode::Projection || graph.payload(node) != index {
        return None;
    }
    Some(graph.resolve(*graph.inputs(node).first()?))
}
