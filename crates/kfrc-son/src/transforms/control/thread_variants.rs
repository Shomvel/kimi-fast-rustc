//! Route known variant predecessors around a merge-and-test diamond.
//!
//! Each edit is planned before mutation. All merge phis, including effects,
//! are split together, and only pure values are cloned. Unknown and poison
//! decisions keep a residual branch; loop-carried roots and unplaceable uses
//! reject the edit.

use kfrc_body::fx_hash::{HashMap, HashSet};
use std::collections::VecDeque;

use kfrc_body::{TypeOracle, TypeShape};

use super::thread_values::{Expression, Merge, Placement, Planner};
use crate::analysis::cfg::ControlFlow;
use crate::analysis::dominators::Dominators;
use crate::analysis::uses::{UseKind, Uses};
use crate::transforms::utils::cfg::CfgEditError;
use crate::{ComparePredicate, FunctionGraph, NodeId, Opcode, ValueType};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct VariantThreadStats {
    pub control_spines_collapsed: usize,
    pub regions_threaded: usize,
    pub branches_removed: usize,
    pub predecessors_threaded: usize,
    pub values_cloned: usize,
    pub dirty_nodes: Vec<NodeId>,
    /// Live graph nodes presented to each Uses/CFG index rebuild.
    pub nodes_indexed: usize,
    /// Candidate roots and pure-cone nodes inspected.
    pub node_visits: usize,
}

pub fn run(graph: &mut FunctionGraph) -> Result<VariantThreadStats, CfgEditError> {
    static EAGER: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    let eager =
        *EAGER.get_or_init(|| std::env::var("KFRC_THREAD_ANALYSIS").as_deref() == Ok("eager"));
    run_with_analysis_policy(graph, eager)
}

fn run_with_analysis_policy(
    graph: &mut FunctionGraph,
    eager: bool,
) -> Result<VariantThreadStats, CfgEditError> {
    let mut stats = VariantThreadStats::default();
    graph.resolve_edges();
    let mut pending: VecDeque<_> = graph
        .ids()
        .filter(|n| graph.op(*n) == Opcode::Region)
        .map(|n| (n, 0))
        .collect();
    let mut analysis = None;
    let mut deferred_cleanup = false;
    // Only newly split regions can expose the next test in a SwitchInt chain.
    while let Some((region, depth)) = pending.pop_front() {
        if graph.is_dead(region) || depth >= 8 {
            continue;
        }
        stats.node_visits += 1;
        if !(2..=64).contains(&graph.inputs(region).len()) {
            continue;
        }
        if analysis.is_none() {
            analysis = Some(Analysis::new(graph, eager, &mut stats.nodes_indexed));
        }
        let index = analysis.as_mut().unwrap();
        if let Some(spine) = TransparentSpine::new(graph, region, &index.uses) {
            stats.dirty_nodes.extend(
                spine
                    .phis
                    .iter()
                    .flat_map(|phi| index.uses.of(*phi).iter().map(|usage| usage.user)),
            );
            spine.apply(graph, region);
            stats.control_spines_collapsed += 1;
            analysis = None;
            pending.push_front((region, depth));
            continue;
        }
        let Some(plan) = Plan::new(graph, region, index, &mut stats) else {
            continue;
        };
        stats.predecessors_threaded += plan.slots.iter().map(Vec::len).sum::<usize>();
        stats.branches_removed += usize::from(plan.residual.is_empty());
        let old_len = graph.len();
        let (cloned, regions, consumers) = plan.apply(graph);
        stats.values_cloned += cloned;
        stats.regions_threaded += 1;
        stats
            .dirty_nodes
            .extend((old_len..graph.len()).map(|i| NodeId(i as u32)));
        stats.dirty_nodes.extend(consumers);
        if simplify_once() {
            deferred_cleanup = true;
        } else {
            super::simplify::run(graph)?;
        }
        analysis = None;
        pending.extend(regions.into_iter().map(|n| (n, depth + 1)));
    }
    if deferred_cleanup {
        super::simplify::run(graph)?;
    }
    stats.dirty_nodes.retain(|node| !graph.is_dead(*node));
    stats.dirty_nodes.sort_unstable();
    stats.dirty_nodes.dedup();
    Ok(stats)
}

/// Transparent spines and dynamic decisions need only reverse edges. Delay
/// reachability and dominance until a known predecessor could actually be
/// threaded. Every edit still discards the complete immutable snapshot.
struct Analysis {
    uses: Uses,
    dominance: Option<(ControlFlow, Dominators)>,
}

impl Analysis {
    fn new(graph: &FunctionGraph, eager: bool, nodes_indexed: &mut usize) -> Self {
        *nodes_indexed += graph.live_count();
        let mut index = Self {
            uses: Uses::new(graph),
            dominance: None,
        };
        if eager {
            index.dominance = Some(Self::build_dominance(graph, nodes_indexed));
        }
        index
    }

    fn build_dominance(
        graph: &FunctionGraph,
        nodes_indexed: &mut usize,
    ) -> (ControlFlow, Dominators) {
        *nodes_indexed += graph.live_count();
        let cfg = ControlFlow::new(graph);
        let dominators = Dominators::new(&cfg);
        (cfg, dominators)
    }
}

/// Clean control once after the threading worklist drains. Planning always
/// rebuilds its Uses/CFG snapshot after an edit; full simplification is not
/// needed to validate the next independent plan. `each` retains the old policy
/// as a measurement control.
fn simplify_once() -> bool {
    static ONCE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ONCE.get_or_init(|| std::env::var("KFRC_THREAD_SIMPLIFY").as_deref() != Ok("each"))
}

// Inlining leaves Jump -> single-entry Region wrappers between a merge and
// its test. Remove only an entirely transparent chain, then rebuild analyses
// and let the original threading plan prove all value/edge placements.
struct TransparentSpine {
    controls: Vec<NodeId>,
    phis: Vec<NodeId>,
    branch: NodeId,
}

impl TransparentSpine {
    fn new(graph: &FunctionGraph, root: NodeId, uses: &Uses) -> Option<Self> {
        let (phis, mut exit) = region_exit(graph, root, uses)?;
        if phis.is_empty() || graph.op(exit) != Opcode::Jump {
            return None;
        }
        let mut spine = Self {
            controls: Vec::new(),
            phis: Vec::new(),
            branch: NodeId::NONE,
        };
        for _ in 0..8 {
            if !graph.inputs(exit).is_empty() {
                return None;
            }
            let [usage] = uses.of(exit) else {
                return None;
            };
            let region = usage.user;
            if usage.kind != UseKind::Operand(0)
                || graph.op(region) != Opcode::Region
                || graph.inputs(region) != [exit]
            {
                return None;
            }
            let (phis, successor) = region_exit(graph, region, uses)?;
            // Identity phis must not define a cycle within this wrapper.
            if phis.iter().any(|phi| phis.contains(&graph.input(*phi, 1))) {
                return None;
            }
            spine.controls.extend([exit, region]);
            spine.phis.extend(phis);
            if matches!(graph.op(successor), Opcode::If | Opcode::VariantSwitch) {
                spine.branch = successor;
                return Some(spine);
            }
            exit = successor;
        }
        None
    }

    fn apply(self, graph: &mut FunctionGraph, root: NodeId) {
        for phi in self.phis {
            graph.replace(phi, graph.input(phi, 1));
        }
        graph.set_control(self.branch, root);
        for control in self.controls {
            graph.remove_node(control);
        }
        graph.resolve_edges();
    }
}

/// All non-phi uses must be the one terminator. An anchored pure operation,
/// fixed observation, extra exit or other use makes the region nontransparent.
fn region_exit(
    graph: &FunctionGraph,
    region: NodeId,
    uses: &Uses,
) -> Option<(Vec<NodeId>, NodeId)> {
    let count = graph.inputs(region).len();
    let mut phis = Vec::new();
    let mut exit = None;
    for usage in uses.of(region) {
        match usage.kind {
            UseKind::Operand(0) if graph.op(usage.user) == Opcode::Phi => {
                if graph.inputs(usage.user).len() != count + 1 {
                    return None;
                }
                phis.push(usage.user);
            }
            UseKind::Control
                if graph.op(usage.user) == Opcode::Phi
                    && graph.inputs(usage.user).first() == Some(&region) => {}
            UseKind::Control
                if matches!(
                    graph.op(usage.user),
                    Opcode::Jump | Opcode::If | Opcode::VariantSwitch
                ) && exit.is_none() =>
            {
                exit = Some(usage.user);
            }
            _ => return None,
        }
    }
    Some((phis, exit?))
}

struct Rewrite {
    user: NodeId,
    slot: usize,
    value: NodeId,
    placement: Placement,
}

struct Plan {
    region: NodeId,
    branch: NodeId,
    leaves: Vec<NodeId>,
    slots: Vec<Vec<usize>>,
    residual: Vec<usize>,
    phis: Vec<NodeId>,
    dependent: HashSet<NodeId>,
    witnesses: HashSet<NodeId>,
    rewrites: Vec<Rewrite>,
    merges: Vec<Merge>,
    expressions: Vec<Expression>,
}

impl Plan {
    fn new(
        graph: &FunctionGraph,
        region: NodeId,
        analysis: &mut Analysis,
        stats: &mut VariantThreadStats,
    ) -> Option<Self> {
        let Analysis { uses, dominance } = analysis;
        let count = graph.inputs(region).len();
        if !(2..=64).contains(&count) {
            return None;
        }
        let (phis, branch) = region_exit(graph, region, uses)?;
        if phis.is_empty() || graph.op(branch) == Opcode::Jump {
            return None;
        }
        let semantic_switch = graph.op(branch) == Opcode::VariantSwitch;
        let cases = if semantic_switch {
            Some(graph.variant_switches.get(graph.payload(branch) as usize)?)
        } else {
            None
        };
        let sides = cases.map_or(2, |cases| cases.len() + 1);
        if sides > 64 {
            return None;
        }
        let mut leaves = vec![NodeId::NONE; sides];
        for usage in uses.of(branch) {
            let node = usage.user;
            let side = graph.payload(node) as usize;
            if usage.kind != UseKind::Operand(0)
                || graph.op(node) != Opcode::Projection
                || graph.value_type(node) != &ValueType::Control
                || side >= sides
                || !leaves[side].is_none()
            {
                return None;
            }
            leaves[side] = node;
        }
        if leaves.iter().any(|n| n.is_none()) {
            return None;
        }
        let condition = *graph.inputs(branch).first()?;
        let mut slots = vec![Vec::new(); sides];
        let mut residual = Vec::new();
        for slot in 0..count {
            let side = if let Some(cases) = cases {
                let value = incoming(graph, condition, region, slot);
                if graph.op(value) != Opcode::MakeVariant {
                    residual.push(slot);
                    continue;
                }
                cases
                    .iter()
                    .position(|&variant| variant == graph.payload(value))
                    .unwrap_or(cases.len())
            } else {
                let Some(value @ 0..=1) = evaluate(graph, condition, region, slot, 0) else {
                    residual.push(slot);
                    continue;
                };
                usize::from(value == 0)
            };
            slots[side].push(slot);
        }
        if residual.len() == count {
            return None;
        }
        // Retire proof edges with their branch. Every routed predecessor
        // constructs the witnessed variant; cloned fields can conservatively
        // use their original total (inactive -> poison) one-input semantics.
        let mut witnesses = HashSet::default();
        for (side, &leaf) in leaves.iter().enumerate() {
            for usage in uses.of(leaf) {
                let witness = usage.user;
                if graph.op(witness) != Opcode::VariantWitness || usage.kind != UseKind::Operand(1)
                {
                    continue;
                }
                let variant = graph.payload(witness);
                if !semantic_switch || graph.input(witness, 0) != condition {
                    return None;
                }
                if slots[side]
                    .iter()
                    .any(|&slot| graph.payload(incoming(graph, condition, region, slot)) != variant)
                {
                    return None;
                }
                witnesses.insert(witness);
            }
        }
        let (cfg, dominators) = dominance
            .get_or_insert_with(|| Analysis::build_dominance(graph, &mut stats.nodes_indexed));
        if !cfg.contains(region) {
            return None;
        }
        if graph
            .inputs(region)
            .iter()
            .any(|pred| dominators.dominates(cfg, region, *pred))
        {
            return None;
        }
        // Discover the complete pure cone, not just the condition: downstream
        // payload fields and effect phis must follow the same predecessor slots.
        let mut dependent: HashSet<_> = phis.iter().copied().collect();
        let mut pending: Vec<_> = dependent.iter().copied().collect();
        pending.sort_unstable();
        while let Some(node) = pending.pop() {
            stats.node_visits += 1;
            for usage in uses.of(node) {
                if usage.user == branch || phis.contains(&usage.user) {
                    continue;
                }
                if cloneable(graph, usage.user) && dependent.insert(usage.user) {
                    if dependent.len() > 128 {
                        return None;
                    }
                    pending.push(usage.user);
                }
            }
        }
        for &witness in &witnesses {
            if uses.of(witness).iter().any(|usage| {
                usage.kind != UseKind::Operand(1)
                    || graph.op(usage.user) != Opcode::VariantField
                    || !dependent.contains(&usage.user)
            }) {
                return None;
            }
        }
        // Partial edits also add a residual projection and Jump per side.
        let side_growth =
            (dependent.len() + phis.len() + 1 + 2 * usize::from(!residual.is_empty())) * sides;
        if side_growth > 512 {
            return None;
        }
        for &phi in &phis {
            if graph.inputs(phi)[1..].iter().any(|n| dependent.contains(n)) {
                return None;
            }
        }
        let mut rewrites = Vec::new();
        let live_sides: Vec<_> = slots
            .iter()
            .map(|side| !side.is_empty() || !residual.is_empty())
            .collect();
        let mut repair = Planner::new(
            graph,
            cfg,
            dominators,
            region,
            &leaves,
            &live_sides,
            &phis,
            &dependent,
            &witnesses,
        );
        let mut ordered: Vec<_> = dependent.iter().copied().collect();
        ordered.sort_unstable();
        for value in ordered {
            for usage in uses.of(value) {
                if dependent.contains(&usage.user)
                    || usage.user == branch
                    || witnesses.contains(&usage.user)
                {
                    continue;
                }
                let UseKind::Operand(slot) = usage.kind else {
                    return None;
                };
                let control = if graph.op(usage.user) == Opcode::Phi {
                    if slot == 0 {
                        return None;
                    }
                    let merge = *graph.inputs(usage.user).first()?;
                    *graph.inputs(merge).get(slot - 1)?
                } else {
                    graph.control(usage.user)
                };
                let before = repair.visits;
                let placement = repair.at(value, control);
                stats.node_visits += repair.visits - before;
                let placement = placement?;
                rewrites.push(Rewrite {
                    user: usage.user,
                    slot,
                    value,
                    placement,
                });
            }
        }
        if side_growth + repair.merges.len() + repair.expressions.len() > 512 {
            return None;
        }
        let merges = repair.merges;
        let expressions = repair.expressions;
        Some(Self {
            region,
            branch,
            leaves,
            slots,
            residual,
            phis,
            dependent,
            witnesses,
            rewrites,
            merges,
            expressions,
        })
    }

    fn apply(self, graph: &mut FunctionGraph) -> (usize, Vec<NodeId>, Vec<NodeId>) {
        let mut maps = vec![HashMap::default(); self.leaves.len()];
        let mut regions = Vec::new();
        let consumers = self.rewrites.iter().map(|r| r.user).collect();
        let residual = !self.residual.is_empty();
        for (side, map) in maps.iter_mut().enumerate() {
            if self.slots[side].is_empty() && !residual {
                continue;
            }
            let mut predecessors: Vec<_> = self.slots[side]
                .iter()
                .map(|i| graph.inputs(self.region)[*i])
                .collect();
            if residual {
                // The old projection is replaced by the side merge below.
                // A fresh projection keeps that merge's residual edge from
                // resolving to itself during replacement normalization.
                let projection = graph.add_node(
                    Opcode::Projection,
                    NodeId::NONE,
                    &[self.branch],
                    graph.payload(self.leaves[side]),
                    ValueType::Control,
                );
                predecessors.push(graph.add_node(
                    Opcode::Jump,
                    projection,
                    &[],
                    0,
                    ValueType::Control,
                ));
            }
            let region = graph.add_node(
                Opcode::Region,
                NodeId::NONE,
                &predecessors,
                0,
                ValueType::Control,
            );
            regions.push(region);
            for &phi in &self.phis {
                let mut values: Vec<_> = self.slots[side]
                    .iter()
                    .map(|i| graph.inputs(phi)[i + 1])
                    .collect();
                if residual {
                    values.push(phi);
                }
                let replacement = if values.len() == 1 {
                    values[0]
                } else {
                    let mut inputs = vec![region];
                    inputs.extend(values);
                    graph.add_node(
                        Opcode::Phi,
                        NodeId::NONE,
                        &inputs,
                        0,
                        graph.value_type(phi).clone(),
                    )
                };
                map.insert(phi, replacement);
            }
            graph.replace(self.leaves[side], region);
        }
        if residual {
            let predecessors: Vec<_> = self
                .residual
                .iter()
                .map(|&slot| graph.inputs(self.region)[slot])
                .collect();
            for &phi in &self.phis {
                let mut inputs = vec![self.region];
                inputs.extend(self.residual.iter().map(|&slot| graph.input(phi, slot + 1)));
                graph.set_inputs(phi, &inputs);
            }
            graph.set_inputs(self.region, &predecessors);
        }
        let merged: Vec<_> = self
            .merges
            .iter()
            .map(|merge| {
                graph.add_node(
                    Opcode::Phi,
                    NodeId::NONE,
                    &[merge.region],
                    0,
                    graph.value_type(merge.value).clone(),
                )
            })
            .collect();
        let mut values = ValueRepair {
            plan: &self,
            maps,
            merged: &merged,
            expressions: vec![NodeId::NONE; self.expressions.len()],
            cloned: 0,
        };
        for (merge, &phi) in self.merges.iter().zip(&merged) {
            let mut inputs = vec![merge.region];
            inputs.extend(
                merge
                    .inputs
                    .iter()
                    .map(|&placement| values.at(graph, merge.value, placement)),
            );
            graph.set_inputs(phi, &inputs);
        }
        for rewrite in &self.rewrites {
            if rewrite.placement != Placement::Unreachable {
                let replacement = values.at(graph, rewrite.value, rewrite.placement);
                graph.set_input(rewrite.user, rewrite.slot, replacement);
            }
        }
        let cloned = values.cloned;
        if !residual {
            graph.remove_node(self.branch);
            graph.remove_node(self.region);
            for node in self.dependent {
                graph.remove_node(node);
            }
        }
        for witness in self.witnesses {
            graph.remove_node(witness);
        }
        graph.resolve_edges();
        (cloned, regions, consumers)
    }
}

/// Materialize side translations and repaired expressions only when used.
/// Repair Phis are allocated first so loop inputs can refer back to them.
struct ValueRepair<'a> {
    plan: &'a Plan,
    maps: Vec<HashMap<NodeId, NodeId>>,
    merged: &'a [NodeId],
    expressions: Vec<NodeId>,
    cloned: usize,
}

impl ValueRepair<'_> {
    fn at(&mut self, graph: &mut FunctionGraph, value: NodeId, placement: Placement) -> NodeId {
        match placement {
            Placement::Original => value,
            Placement::Side(side) => clone_value(
                graph,
                value,
                &self.plan.dependent,
                &self.plan.witnesses,
                &mut self.maps[side],
                &mut self.cloned,
            ),
            Placement::Merge(index) => self.merged[index],
            Placement::Expression(index) => {
                if !self.expressions[index].is_none() {
                    return self.expressions[index];
                }
                let original = self.plan.expressions[index].value;
                let mut inputs = Vec::with_capacity(self.plan.expressions[index].inputs.len());
                for slot in 0..self.plan.expressions[index].inputs.len() {
                    let (input, placement) = self.plan.expressions[index].inputs[slot];
                    inputs.push(self.at(graph, input, placement));
                }
                let replacement = graph.add_node(
                    graph.op(original),
                    NodeId::NONE,
                    &inputs,
                    graph.payload(original),
                    graph.value_type(original).clone(),
                );
                self.expressions[index] = replacement;
                self.cloned += 1;
                replacement
            }
            // Reachability repair removes this operand's predecessor before
            // verification observes the retired original definition.
            Placement::Unreachable => value,
        }
    }
}

pub(super) fn cloneable(graph: &FunctionGraph, node: NodeId) -> bool {
    graph.control(node).is_none()
        && matches!(
            graph.op(node),
            Opcode::Aggregate
                | Opcode::Repeat
                | Opcode::Field
                | Opcode::InsertField
                | Opcode::MakeVariant
                | Opcode::VariantOf
                | Opcode::VariantField
                | Opcode::Discriminant
                | Opcode::WidePtr
                | Opcode::Add
                | Opcode::Sub
                | Opcode::Mul
                | Opcode::BitAnd
                | Opcode::BitOr
                | Opcode::BitXor
                | Opcode::BitNot
                | Opcode::Negate
                | Opcode::Compare
                | Opcode::IntCast
                | Opcode::ShiftLeft
                | Opcode::ShiftRightLogical
                | Opcode::ShiftRightArithmetic
                | Opcode::BitCount
                | Opcode::ByteSwap
                | Opcode::BitReverse
                | Opcode::FunnelShift
                | Opcode::SaturatingAdd
                | Opcode::SaturatingSub
                | Opcode::PointerCast
                | Opcode::FieldAddr
                | Opcode::VariantFieldAddr
                | Opcode::IndexAddr
                | Opcode::PtrOffset
        )
}

fn clone_value(
    graph: &mut FunctionGraph,
    node: NodeId,
    dependent: &HashSet<NodeId>,
    witnesses: &HashSet<NodeId>,
    map: &mut HashMap<NodeId, NodeId>,
    cloned: &mut usize,
) -> NodeId {
    if !dependent.contains(&node) {
        return node;
    }
    if let Some(&value) = map.get(&node) {
        return value;
    }
    let mut original = graph.inputs(node).to_vec();
    if graph.op(node) == Opcode::VariantField
        && original
            .get(1)
            .is_some_and(|proof| witnesses.contains(proof))
    {
        original.truncate(1);
    }
    let inputs: Vec<_> = original
        .into_iter()
        .map(|n| clone_value(graph, n, dependent, witnesses, map, cloned))
        .collect();
    let result = graph.add_node(
        graph.op(node),
        NodeId::NONE,
        &inputs,
        graph.payload(node),
        graph.value_type(node).clone(),
    );
    map.insert(node, result);
    *cloned += 1;
    result
}

fn incoming(graph: &FunctionGraph, mut node: NodeId, region: NodeId, slot: usize) -> NodeId {
    for _ in 0..64 {
        if graph.op(node) != Opcode::Phi || graph.inputs(node).first() != Some(&region) {
            break;
        }
        node = graph.inputs(node)[slot + 1];
    }
    node
}

fn normalize(graph: &FunctionGraph, node: NodeId, value: u128) -> Option<u128> {
    let bits = match graph.oracle.shape(graph.rust_type(node)?) {
        TypeShape::Bool => 1,
        TypeShape::Char => 32,
        TypeShape::Int { width, .. } => width.bits(64),
        _ => return None,
    };
    Some(value & (u128::MAX >> (128 - u32::from(bits))))
}

fn evaluate(
    graph: &FunctionGraph,
    node: NodeId,
    region: NodeId,
    slot: usize,
    depth: usize,
) -> Option<u128> {
    if depth > 64 {
        return None;
    }
    let node = incoming(graph, node, region, slot);
    let inputs = graph.inputs(node);
    let eval = |n| evaluate(graph, n, region, slot, depth + 1);
    match graph.op(node) {
        Opcode::Const => normalize(graph, node, graph.constant_value(node)),
        Opcode::VariantOf | Opcode::Discriminant => {
            let source = incoming(graph, *inputs.first()?, region, slot);
            if graph.op(source) != Opcode::MakeVariant {
                return None;
            }
            let variant = graph.payload(source);
            let value = if graph.op(node) == Opcode::VariantOf {
                u128::from(variant)
            } else {
                graph
                    .oracle
                    .semantic()
                    .discriminant(graph.rust_type(source)?, variant)?
            };
            normalize(graph, node, value)
        }
        Opcode::Compare => {
            let left = eval(*inputs.first()?)?;
            let right = eval(*inputs.get(1)?)?;
            // Equality needs no signedness interpretation and covers emitted
            // SwitchInt chains. Ordering decisions remain with the scalar pass.
            Some(u128::from(
                match ComparePredicate::from_payload(graph.payload(node)) {
                    ComparePredicate::Equal => left == right,
                    ComparePredicate::NotEqual => left != right,
                    _ => return None,
                },
            ))
        }
        Opcode::BitNot => normalize(graph, node, !eval(*inputs.first()?)?),
        _ => None,
    }
}

#[cfg(test)]
#[path = "thread_variants_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "thread_partial_tests.rs"]
mod partial_tests;
