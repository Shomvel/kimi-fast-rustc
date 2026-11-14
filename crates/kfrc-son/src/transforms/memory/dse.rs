//! Bounded complete-overwrite DSE on control/effect paths.
//!
//! A deleted store needs a proven writable address, total operand evaluation
//! and a complete scalar/Unit encoding. Storing poison remains legal.
//! Every path must overwrite every byte before a read, lifetime
//! change, call, observation or trap. Only already-defined two-way conditions
//! are crossed. No access, requirement, or value evaluation changes its path.
#[cfg(test)]
use crate::ValueType;
use crate::analysis::memory::MemoryLocations;
use crate::analysis::uses::{UseKind, Uses};
use crate::{FunctionGraph, GraphLevel, NodeId, Opcode};

#[path = "dse_branches.rs"]
mod branches;
#[path = "dse_values.rs"]
mod values;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DseStats {
    pub stores_removed: usize,
    pub node_visits: usize,
    pub walk_steps: usize,
    pub budget_bailouts: usize,
    pub branches_visited: usize,
    pub stores_removed_across_branches: usize,
    pub stores_removed_using_byte_ranges: usize,
    pub dirty_nodes: Vec<NodeId>,
}

pub fn run(graph: &mut FunctionGraph) -> DseStats {
    let mut stats = DseStats::default();
    // Access permissions/layout and the proof below describe S, never generic
    // R or representation-generated L stores.
    if graph.level != GraphLevel::Specialized {
        return stats;
    }
    graph.resolve_edges();
    let stores: Vec<_> = graph
        .ids()
        .filter(|&node| graph.op(node) == Opcode::StoreValue)
        .collect();
    if stores.len() < 2 {
        return stats;
    }
    let uses = Uses::new(graph);
    let mut locations = MemoryLocations::new(graph);
    let mut store_safety = values::StoreSafety::new(graph);
    let mut fuel = graph.len().saturating_mul(8).clamp(64, 100_000);
    let mut dead = Vec::new();
    for old in stores {
        let address = graph.resolve(graph.input(old, 1));
        if !locations.valid_store(graph, address, old)
            && !validated_before(graph, old, &mut locations, &mut fuel, &mut stats)
        {
            continue;
        }
        if !store_safety.operands_and_encoding(graph, old, &mut fuel, &mut stats) {
            continue;
        }
        if let Some(proof) = branches::complete_overwrite(
            graph,
            &uses,
            old,
            &mut locations,
            &mut store_safety,
            &mut fuel,
            &mut stats,
        ) {
            dead.push((old, proof.branched, proof.byte_coverage));
        }
    }
    // Discover on an immutable snapshot, then erase. Even consecutive dead
    // stores redirect through forwarding entries and retain the final writer.
    for (store, branched, byte_coverage) in dead {
        let consumers = uses.of(store);
        if consumers.iter().any(|usage| {
            usage.kind != UseKind::Operand(0)
                || graph.op(usage.user) != Opcode::Projection
                || graph.payload(usage.user) > 1
        }) {
            continue;
        }
        for projection in consumers.iter().map(|usage| usage.user) {
            let input = if graph.payload(projection) == 0 {
                graph.control(store)
            } else {
                graph.input(store, 0)
            };
            stats
                .dirty_nodes
                .extend(uses.of(projection).iter().map(|usage| usage.user));
            graph.replace(projection, input);
        }
        graph.remove_node(store);
        stats.stores_removed += 1;
        stats.stores_removed_across_branches += usize::from(branched);
        stats.stores_removed_using_byte_ranges += usize::from(byte_coverage);
    }
    graph.resolve_edges();
    stats.node_visits += locations.node_visits;
    stats.budget_bailouts += locations.budget_bailouts;
    stats.dirty_nodes.retain(|&node| !graph.is_dead(node));
    stats.dirty_nodes.sort_unstable();
    stats.dirty_nodes.dedup();
    stats
}

/// Keep an earlier access that already checked the same writable range.
/// Only intervening stores are crossed, so no call or lifetime change can
/// invalidate that access. A raw address's first store always remains unless
/// it independently has the static proof above.
fn validated_before(
    graph: &FunctionGraph,
    store: NodeId,
    locations: &mut MemoryLocations,
    fuel: &mut usize,
    stats: &mut DseStats,
) -> bool {
    let address = graph.resolve(graph.input(store, 1));
    let mut current = store;
    for _ in 0..32 {
        if *fuel == 0 {
            stats.budget_bailouts += 1;
            return false;
        }
        *fuel -= 1;
        stats.walk_steps += 1;
        let control = graph.resolve(graph.control(current));
        let effect = graph.resolve(graph.input(current, 0));
        if graph.op(control) != Opcode::Projection
            || graph.payload(control) != 0
            || graph.op(effect) != Opcode::Projection
            || graph.payload(effect) != 1
        {
            return false;
        }
        let previous = graph.resolve(graph.input(control, 0));
        if graph.resolve(graph.input(effect, 0)) != previous
            || graph.op(previous) != Opcode::StoreValue
        {
            return false;
        }
        if locations.same_typed_place(graph, address, graph.input(previous, 1)) {
            return true;
        }
        current = previous;
    }
    stats.budget_bailouts += 1;
    false
}

#[cfg(test)]
#[path = "dse_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "dse_branch_tests.rs"]
mod branch_tests;
