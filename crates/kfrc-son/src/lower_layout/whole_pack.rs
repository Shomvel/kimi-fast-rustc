//! Recover one whole integer from sequential byte reads or synchronized Phis.
//! Only a complete pack can collapse independently undefined bytes to one
//! scalar poison bit. Reads stay on their original guarded fixed edge, before
//! any subsequent effect.
use super::*;
use crate::analysis::uses::{UseKind, Uses};

mod index;
mod loads;
mod pack;
mod transmute;

#[cfg(test)]
mod tests;

const FUEL: usize = 262_144;

impl LayoutLowering<'_> {
    pub(super) fn pack_array_transmute(&mut self, source: &Value, target: TyId) -> Option<Value> {
        let packed = transmute::pack(
            self.graph,
            source,
            target,
            self.options.max_unrolled_memory_bytes,
        )?;
        self.stats.whole_array_transmutes += 1;
        Some(packed)
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct Stats {
    pub groups: usize,
    /// Net reduction after replacing N byte loads with one word load.
    pub byte_loads_removed: usize,
    /// Net reduction after replacing N byte Phis with one word Phi.
    pub payload_phis_removed: usize,
    pub budget_exhausted: bool,
}

struct Fuel {
    left: usize,
    exhausted: bool,
}

impl Fuel {
    fn take(&mut self, count: usize) -> Option<()> {
        self.left = self.left.checked_sub(count).or_else(|| {
            self.exhausted = true;
            None
        })?;
        Some(())
    }
}

pub(super) fn run(graph: &mut FunctionGraph, max_bytes: u64) -> Stats {
    run_with_fuel(graph, max_bytes, FUEL)
}

fn run_with_fuel(graph: &mut FunctionGraph, max_bytes: u64, limit: usize) -> Stats {
    if graph.level != GraphLevel::Layout || max_bytes < 2 {
        return Stats::default();
    }
    let mut fuel = Fuel {
        left: limit,
        exhausted: false,
    };
    // Most functions have no pack at all. Avoid allocating a dense use index
    // there, while charging the scan to the same all-or-nothing budget.
    let mut roots = Vec::new();
    for node in graph.ids() {
        if fuel.take(1).is_none() {
            return Stats {
                budget_exhausted: true,
                ..Stats::default()
            };
        }
        if graph.op(node) == Opcode::BitOr {
            roots.push(node);
        }
    }
    if roots.is_empty() {
        return Stats::default();
    }
    let Some(uses) = index::uses(graph, &mut fuel) else {
        return Stats {
            budget_exhausted: fuel.exhausted,
            ..Stats::default()
        };
    };
    // All analysis precedes mutation. Plans own disjoint byte/pack cones;
    // shared pointer roots, effects outside a cone and scalar consumers survive.
    let mut plans = Vec::new();
    let mut claimed = HashSet::default();
    for root in roots {
        if fuel.take(1).is_none() {
            break;
        }
        let Some(pack) = pack::plan(graph, &uses, root, max_bytes, &mut fuel) else {
            continue;
        };
        let Some(plan) = loads::plan(graph, &uses, pack, &mut fuel) else {
            continue;
        };
        if plan.owned.iter().any(|node| claimed.contains(node)) {
            continue;
        }
        claimed.extend(plan.owned.iter().copied());
        plans.push(plan);
    }
    if fuel.exhausted {
        return Stats {
            budget_exhausted: true,
            ..Stats::default()
        };
    }
    let mut stats = Stats::default();
    for plan in plans {
        stats.groups += 1;
        stats.byte_loads_removed += plan.loads.len() - 1;
        stats.payload_phis_removed += plan.phis_removed();
        plan.apply(graph);
    }
    if stats.groups != 0 {
        graph.compact();
    }
    stats
}
