//! Publish private scalar storage as L values before cleanup and threading.
//! Exact, disjoint typed lanes are sufficient for ordinary struct fields.
//! Overlapping representations, address merges and ABI transfers keep storage.
//! A lane with no reaching store is Undefined, preserving the load's poison
//! without imposing initialization on paths which never observe that lane.
use super::*;
use crate::analysis::{
    cfg::ControlFlow,
    uses::{UseKind, Uses},
};

mod collect;
mod phis;
mod states;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Lane {
    offset: u64,
    bytes: u64,
    ty: TyId,
}

struct Slot {
    lanes: Vec<Lane>,
    // Lifetime markers have no lane and kill the entire allocation's state.
    accesses: BTreeMap<NodeId, Option<usize>>,
}

/// Stores after SlotDead remain observable undefined accesses in the L
/// interpreter. Refuse the whole slot unless every such path meets SlotLive.
fn live(graph: &FunctionGraph, cfg: &ControlFlow, slot: &Slot, fuel: &mut usize) -> bool {
    let mut pending: Vec<_> = slot
        .accesses
        .keys()
        .filter(|&&node| graph.op(node) == Opcode::SlotDead)
        .flat_map(|&node| cfg.successors(node).iter().copied())
        .collect();
    let mut seen = HashSet::default();
    while let Some(node) = pending.pop() {
        if !seen.insert(node) {
            continue;
        }
        let Some(remaining) = fuel.checked_sub(1) else {
            return false;
        };
        *fuel = remaining;
        if slot.accesses.contains_key(&node) {
            match graph.op(node) {
                Opcode::SlotLive => continue,
                Opcode::Load | Opcode::Store => return false,
                _ => {}
            }
        }
        pending.extend_from_slice(cfg.successors(node));
    }
    true
}

pub(super) fn run(graph: &mut FunctionGraph) -> usize {
    debug_assert_eq!(graph.level, GraphLevel::Layout);
    graph.resolve_edges();
    let mut roots = BTreeMap::<u32, Vec<NodeId>>::new();
    for node in graph
        .ids()
        .filter(|&node| graph.op(node) == Opcode::StackSlot)
    {
        roots.entry(graph.payload(node)).or_default().push(node);
    }
    if roots.is_empty() {
        return 0;
    }
    let uses = Uses::new(graph);
    let cfg = ControlFlow::new(graph);
    // The lazy value builder relies on explicit Regions at every join.
    if cfg.order.iter().enumerate().any(|(index, &node)| {
        node != graph.start
            && graph.op(node) != Opcode::Region
            && cfg.predecessors[index].len() != 1
    }) {
        return 0;
    }
    let mut fuel = 100_000usize;
    let mut expansion = 65_536usize;
    let mut rewrites = Vec::new();
    let mut phis = Vec::new();
    let mut promoted = 0;
    for (identity, roots) in roots {
        let Some(slot) = collect::slot(graph, &uses, &cfg, identity, &roots, &mut fuel) else {
            continue;
        };
        // Each (lane, control) is visited once by lazy SSA construction.
        // Reserve its worst-case size before appending any node.
        let Some(remaining) = expansion.checked_sub(cfg.order.len() * slot.lanes.len()) else {
            continue;
        };
        if !live(graph, &cfg, &slot, &mut fuel) {
            continue;
        }
        expansion = remaining;
        for (lane, description) in slot.lanes.iter().enumerate() {
            let mut state = states::Values::new(graph, &cfg, &slot, lane, description.ty);
            for (&node, &access) in &slot.accesses {
                if access == Some(lane) && state.graph.op(node) == Opcode::Load {
                    let value = state.read(state.graph.control(node));
                    rewrites.push((node, Some(value)));
                }
            }
            phis.extend(state.phis);
        }
        rewrites.extend(
            slot.accesses
                .keys()
                .filter(|&&node| graph.op(node) != Opcode::Load)
                .map(|&node| (node, None)),
        );
        promoted += 1;
    }
    phis::simplify(graph, &phis, 65_536);
    // All values use the original CFG; only after planning every slot do we
    // bypass control/effect projections. Calls and other fixed work stay put.
    for (node, value) in rewrites {
        for usage in uses.of(node) {
            let replacement = match graph.payload(usage.user) {
                0 => graph.control(node),
                1 => graph.input(node, 0),
                2 => value.expect("load replacement"),
                _ => unreachable!("checked tuple use"),
            };
            graph.replace(usage.user, replacement);
        }
        graph.remove_node(node);
    }
    graph.resolve_edges();
    if promoted != 0 {
        graph.compact();
    }
    promoted
}

#[cfg(test)]
mod tests;
