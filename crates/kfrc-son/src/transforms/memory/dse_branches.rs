//! An all-path overwrite proof. Failure never publishes partial coverage.
use super::DseStats;
use super::values::StoreSafety;
use crate::analysis::memory::{MemoryLocation, MemoryLocations};
use crate::analysis::uses::{UseKind, Uses};
use crate::{FunctionGraph, NodeId, Opcode, ValueType};
use kfrc_body::{TypeOracle, TypeShape};

const MAX_STATES: usize = 64;
const MAX_BRANCHES: usize = 8;

pub(super) struct CompleteOverwrite {
    pub(super) branched: bool,
    pub(super) byte_coverage: bool,
}

pub(super) fn complete_overwrite(
    graph: &FunctionGraph,
    uses: &Uses,
    old: NodeId,
    locations: &mut MemoryLocations,
    store_safety: &mut StoreSafety,
    fuel: &mut usize,
    stats: &mut DseStats,
) -> Option<CompleteOverwrite> {
    let address = graph.resolve(graph.input(old, 1));
    let place = locations.location(graph, address)?;
    // StoreSafety admitted a complete <=64-bit scalar or Unit. The ninth bit
    // keeps Unit on the existing exact-place proof, rather than treating an
    // empty interval as coverage before any later access executes.
    if place.size > 8 {
        return None;
    }
    let remaining = if place.size == 0 {
        1 << 8
    } else {
        (1 << place.size) - 1
    };
    let (control, effect) = store_outputs(graph, uses, old)?;
    let mut pending = vec![(control, effect, remaining)];
    let mut seen = Vec::new();
    let mut effects = Vec::new();
    let mut covered = Vec::new();
    let mut branches = 0;
    let mut byte_coverage = false;
    while let Some((control, effect, remaining)) = pending.pop() {
        if seen.len() == MAX_STATES || !charge(fuel, stats) {
            if seen.len() == MAX_STATES {
                stats.budget_bailouts += 1;
            }
            return None;
        }
        if seen.contains(&(control, effect)) {
            return None;
        }
        seen.push((control, effect));
        // All uses matter, including anchored Div/Rem, witnesses and operand
        // uses. Filtering only fixed control successors would miss traps.
        let [successor] = uses.of(control) else {
            return None;
        };
        if successor.kind != UseKind::Control {
            return None;
        }
        let next = successor.user;
        match graph.op(next) {
            Opcode::StoreValue => {
                if graph.resolve(graph.input(next, 0)) != effect
                    || !store_safety.operands_and_encoding(graph, next, fuel, stats)
                {
                    return None;
                }
                if !effects.contains(&effect) {
                    effects.push(effect);
                }
                covered.push((effect, next));
                let next_address = graph.resolve(graph.input(next, 1));
                if locations.same_typed_place(graph, address, next_address) {
                    // The old access was already proved valid (or checked by
                    // an earlier retained store). Exact typed-place equality
                    // preserves its range/alignment; no crossed node can end
                    // that allocation's lifetime or revoke its permission.
                    continue;
                }
                // Byte coverage alone does not make an access non-trapping.
                // Prove each retyped/partial store valid at the old position:
                // the entire prefix rejects calls and lifetime changes, so
                // this fact also holds on either branch when it executes.
                if !locations.valid_store(graph, next_address, old) {
                    return None;
                }
                let next_remaining = if let Some(written) = locations.location(graph, next_address)
                    && place.root == written.root
                {
                    let updated = remaining_after(place, written, remaining, fuel, stats)?;
                    byte_coverage |= updated != remaining;
                    updated
                } else if locations.disjoint(graph, address, next_address) {
                    remaining
                } else {
                    return None;
                };
                if next_remaining != 0 {
                    let (control, effect) = store_outputs(graph, uses, next)?;
                    pending.push((control, effect, next_remaining));
                }
            }
            Opcode::If => {
                if !defined_condition(graph, graph.resolve(graph.input(next, 0))) {
                    return None;
                }
                if branches == MAX_BRANCHES {
                    stats.budget_bailouts += 1;
                    return None;
                }
                let exits = branch_exits(graph, uses, next)?;
                branches += 1;
                stats.branches_visited += 1;
                pending.extend(exits.map(|exit| (exit, effect, remaining)));
            }
            Opcode::Jump => {
                // A single-predecessor Region without Phis only changes the
                // control token. Merges, loops and value/effect Phis stop us.
                let [usage] = uses.of(next) else {
                    return None;
                };
                let region = usage.user;
                if usage.kind != UseKind::Operand(0)
                    || graph.op(region) != Opcode::Region
                    || graph.inputs(region) != [next]
                {
                    return None;
                }
                pending.push((region, effect, remaining));
            }
            _ => return None,
        }
    }
    // Branches may share an incoming effect. Accept it only when every use is
    // a store edge covered above; a bypass consumer or an effect Phi rejects
    // the entire candidate even if both direct control arms were covered.
    for effect in effects {
        if uses.of(effect).len() > MAX_STATES {
            stats.budget_bailouts += 1;
            return None;
        }
        for usage in uses.of(effect) {
            if !charge(fuel, stats)
                || usage.kind != UseKind::Operand(0)
                || !covered.contains(&(effect, usage.user))
            {
                return None;
            }
        }
    }
    Some(CompleteOverwrite {
        branched: branches != 0,
        byte_coverage,
    })
}

/// Both encodings are complete scalars, so every written byte replaces the
/// old byte's bits and initialization state. Unlike typed load forwarding,
/// this proof neither decodes a new type nor synthesizes a stored value.
fn remaining_after(
    old: MemoryLocation,
    written: MemoryLocation,
    mut remaining: u16,
    fuel: &mut usize,
    stats: &mut DseStats,
) -> Option<u16> {
    let end = written.offset.checked_add(written.size)?;
    for byte in 0..old.size {
        if !charge(fuel, stats) {
            return None;
        }
        let offset = old.offset.checked_add(byte)?;
        if written.offset <= offset && offset < end {
            remaining &= !(1 << byte);
        }
    }
    Some(remaining)
}

fn charge(fuel: &mut usize, stats: &mut DseStats) -> bool {
    if *fuel == 0 {
        stats.budget_bailouts += 1;
        return false;
    }
    *fuel -= 1;
    stats.walk_steps += 1;
    true
}

fn defined_condition(graph: &FunctionGraph, condition: NodeId) -> bool {
    if !graph
        .rust_type(condition)
        .is_some_and(|ty| matches!(graph.oracle.shape(ty), TypeShape::Bool))
    {
        return false;
    }
    match graph.op(condition) {
        // Semantic entry checks every argument for poison before any store.
        Opcode::Projection => {
            graph.payload(condition) >= 2 && graph.resolve(graph.input(condition, 0)) == graph.start
        }
        Opcode::Const => graph.constant_value(condition) <= 1,
        _ => false,
    }
}

fn store_outputs(graph: &FunctionGraph, uses: &Uses, store: NodeId) -> Option<(NodeId, NodeId)> {
    if uses.of(store).len() != 2 {
        return None;
    }
    Some((
        projection(graph, uses, store, 0, &ValueType::Control)?,
        projection(graph, uses, store, 1, &ValueType::Effect)?,
    ))
}

fn branch_exits(graph: &FunctionGraph, uses: &Uses, branch: NodeId) -> Option<[NodeId; 2]> {
    if uses.of(branch).len() != 2 {
        return None;
    }
    Some([
        projection(graph, uses, branch, 0, &ValueType::Control)?,
        projection(graph, uses, branch, 1, &ValueType::Control)?,
    ])
}

fn projection(
    graph: &FunctionGraph,
    uses: &Uses,
    source: NodeId,
    index: u32,
    ty: &ValueType,
) -> Option<NodeId> {
    let mut projections = uses.of(source).iter().filter(|usage| {
        usage.kind == UseKind::Operand(0)
            && graph.op(usage.user) == Opcode::Projection
            && graph.payload(usage.user) == index
            && graph.value_type(usage.user) == ty
    });
    let projection = projections.next()?.user;
    projections.next().is_none().then_some(projection)
}
