//! Attach provenance-backed Local/Const labels without changing effect order.
//! Recompute after layout creates calls or changes allocation identities; a
//! semantic Local proof is not automatically a proof about the resulting L.

use std::collections::HashSet;

use crate::analysis::memory::MemoryFacts;
use crate::{FunctionGraph, MemoryPartition, Opcode};

#[derive(Clone, Copy, Debug, Default)]
pub struct PartitionStats {
    pub local_accesses: usize,
    pub const_loads: usize,
    pub node_visits: usize,
    pub edge_visits: usize,
}

pub fn run(graph: &mut FunctionGraph, facts: &MemoryFacts) -> PartitionStats {
    let mut stats = PartitionStats {
        node_visits: facts.node_visits,
        edge_visits: facts.edge_visits,
        ..PartitionStats::default()
    };
    let immutable: HashSet<_> = graph
        .data_objects
        .iter()
        .filter(|object| !object.writable && !object.thread_local)
        .map(|object| object.symbol.clone())
        .collect();
    let nodes: Vec<_> = graph.ids().collect();
    for node in nodes {
        stats.node_visits += 1;
        if !matches!(
            graph.op(node),
            Opcode::LoadValue
                | Opcode::StoreValue
                | Opcode::Load
                | Opcode::Store
                | Opcode::SetVariant
                | Opcode::SlotLive
                | Opcode::SlotDead
                | Opcode::CopyNonOverlapping
        ) {
            continue;
        }
        let root = facts.root(graph.resolve(graph.input(node, 1)));
        let partition = if graph.op(node) == Opcode::CopyNonOverlapping {
            // One payload cannot express both source and destination chains
            // (F60). Retain Unknown until a dual-partition copy is implemented.
            MemoryPartition::Unknown
        } else if let Some(local) = root.and_then(|root| facts.local_identity(root))
            && facts.is_unescaped_local(local)
        {
            stats.local_accesses += 1;
            MemoryPartition::Local { local }
        } else if matches!(graph.op(node), Opcode::LoadValue | Opcode::Load)
            && let Some(root) = root
            && graph.op(root) == Opcode::SymbolAddress
        {
            let symbol = graph.payload(root);
            let name = &graph.symbols[symbol as usize].0;
            if immutable.contains(name) {
                stats.const_loads += 1;
                MemoryPartition::Const { symbol }
            } else {
                MemoryPartition::Unknown
            }
        } else {
            MemoryPartition::Unknown
        };
        let payload = graph.intern_memory_partition(partition);
        if payload != graph.payload(node) {
            graph.set_payload(node, payload);
        }
    }
    stats
}
