//! Compile-time variant witnesses. A false proof is a compiler error, never UB.
//! Proof edges participate in graph maintenance, not in machine value demand.
use crate::analysis::{
    cfg::{ControlFlow, is_anchored_value, is_control_flow},
    dominators::Dominators,
    uses::{UseKind, Uses},
};
use crate::{ComparePredicate, FunctionGraph, NodeId, Opcode};
use std::collections::{HashMap, HashSet};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WitnessStats {
    pub witnesses_checked: usize,
    pub node_visits: usize,
    pub consumer_sites: usize,
}

pub fn variant_on_edge(graph: &FunctionGraph, value: NodeId, edge: NodeId) -> Option<u32> {
    if graph.op(edge) != Opcode::Projection {
        return None;
    }
    let branch = graph.input(edge, 0);
    if graph.op(branch) != Opcode::VariantSwitch
        || graph.resolve(graph.input(branch, 0)) != graph.resolve(value)
    {
        return None;
    }
    let cases = graph.variant_switches.get(graph.payload(branch) as usize)?;
    let slot = graph.payload(edge) as usize;
    if slot < cases.len() {
        return Some(cases[slot]);
    }
    if slot != cases.len() {
        return None;
    }
    let count = graph
        .oracle
        .semantic()
        .variant_count(graph.rust_type(value)?)?;
    let mut remaining = (0..count as u32).filter(|variant| !cases.contains(variant));
    let variant = remaining.next()?;
    remaining.next().is_none().then_some(variant)
}

/// A predicate known on a particular control exit. This records only the
/// Boolean proposition: `ptr != 0` says nothing about validity for dereference.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScopedPredicate {
    pub predicate: NodeId,
    pub truth: bool,
    pub proof: NodeId,
    pub origin: PredicateOrigin,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PredicateOrigin {
    Branch,
    Assumption,
}

fn predicate_on_edge(graph: &FunctionGraph, edge: NodeId) -> Option<ScopedPredicate> {
    if graph.op(edge) != Opcode::Projection {
        return None;
    }
    let source = graph.input(edge, 0);
    let (predicate, truth, origin) = match (graph.op(source), graph.payload(edge)) {
        (Opcode::If, slot @ 0..=1) => (graph.input(source, 0), slot == 0, PredicateOrigin::Branch),
        (Opcode::Assume, 0) => (graph.input(source, 1), true, PredicateOrigin::Assumption),
        _ => return None,
    };
    Some(ScopedPredicate {
        predicate,
        truth,
        proof: edge,
        origin,
    })
}

fn predicate_variant(graph: &FunctionGraph, value: NodeId, fact: ScopedPredicate) -> Option<u32> {
    let predicate = fact.predicate;
    if graph.op(predicate) != Opcode::Compare
        || !((graph.payload(predicate) == ComparePredicate::Equal as u32 && fact.truth)
            || (graph.payload(predicate) == ComparePredicate::NotEqual as u32 && !fact.truth))
    {
        return None;
    }
    for (query, constant) in [
        (graph.input(predicate, 0), graph.input(predicate, 1)),
        (graph.input(predicate, 1), graph.input(predicate, 0)),
    ] {
        if graph.op(query) == Opcode::VariantOf
            && graph.resolve(graph.input(query, 0)) == graph.resolve(value)
            && graph.op(constant) == Opcode::Const
        {
            return u32::try_from(graph.constant_value(constant)).ok();
        }
    }
    None
}

/// Scoped facts for one immutable CFG snapshot. Consumers must query at their
/// actual execution control, including Phi predecessor edges. A graph edit
/// invalidates this cache. Loop traversal has no budget-based success fallback.
pub struct PredicateFacts<'a> {
    graph: &'a FunctionGraph,
    cfg: &'a ControlFlow,
    dominators: &'a Dominators,
    cyclic: bool,
    anchors: HashMap<NodeId, Vec<NodeId>>,
    instances: HashMap<(NodeId, NodeId, NodeId), bool>,
    stats: WitnessStats,
}

impl<'a> PredicateFacts<'a> {
    pub fn new(graph: &'a FunctionGraph, cfg: &'a ControlFlow, dominators: &'a Dominators) -> Self {
        let cyclic = cfg
            .predecessors
            .iter()
            .enumerate()
            .any(|(index, preds)| preds.iter().any(|&pred| pred >= index));
        Self {
            graph,
            cfg,
            dominators,
            cyclic,
            anchors: HashMap::new(),
            instances: HashMap::new(),
            stats: WitnessStats {
                node_visits: cfg.order.len(),
                ..WitnessStats::default()
            },
        }
    }

    pub fn stats(&self) -> WitnessStats {
        self.stats
    }

    /// Returns a fact only when its exit dominates `at` and the predicate's
    /// dynamic inputs have not been redefined since that exit was traversed.
    pub fn predicate_at(&mut self, proof: NodeId, at: NodeId) -> Option<ScopedPredicate> {
        let fact = predicate_on_edge(self.graph, proof)?;
        self.same_instance(fact.predicate, proof, at)
            .then_some(fact)
    }

    fn same_instance(&mut self, value: NodeId, proof: NodeId, at: NodeId) -> bool {
        let value = self.graph.resolve(value);
        if !self.dominators.dominates(self.cfg, proof, at) {
            return false;
        }
        if let Some(&valid) = self.instances.get(&(value, proof, at)) {
            return valid;
        }
        let anchors = self
            .anchors
            .entry(value)
            .or_insert_with(|| definition_anchors(self.graph, value, &mut self.stats));
        let valid = anchors
            .iter()
            .all(|&anchor| self.dominators.dominates(self.cfg, anchor, proof))
            && unchanged_until(self.cfg, self.cyclic, proof, at, anchors, &mut self.stats);
        self.instances.insert((value, proof, at), valid);
        valid
    }
}

/// Dynamic definitions which can change this source value on another iteration.
fn definition_anchors(
    graph: &FunctionGraph,
    source: NodeId,
    stats: &mut WitnessStats,
) -> Vec<NodeId> {
    let mut pending = vec![source];
    let mut seen = HashSet::new();
    let mut anchors = Vec::new();
    while let Some(node) = pending.pop() {
        let node = graph.resolve(node);
        if !seen.insert(node) {
            continue;
        }
        stats.node_visits += 1;
        match graph.op(node) {
            Opcode::Phi => anchors.push(graph.input(node, 0)),
            Opcode::VariantField => pending.push(graph.input(node, 0)),
            Opcode::VariantWitness => {}
            Opcode::Projection => {
                let origin = graph.input(node, 0);
                if graph.op(origin).is_fixed() {
                    anchors.push(origin);
                } else {
                    pending.push(origin);
                }
            }
            op if is_anchored_value(op) => anchors.push(graph.control(node)),
            Opcode::Const
            | Opcode::Undefined
            | Opcode::LocalSlot
            | Opcode::StackSlot
            | Opcode::SymbolAddress => {}
            _ => pending.extend_from_slice(graph.inputs(node)),
        }
    }
    anchors.sort_unstable();
    anchors.dedup();
    anchors
}

/// Follow only business-value consumers. A Phi consumes its operand on that
/// incoming edge; the resulting Phi is a new snapshot, not a late re-read.
fn execution_sites(
    graph: &FunctionGraph,
    uses: &Uses,
    field: NodeId,
    stats: &mut WitnessStats,
) -> Vec<NodeId> {
    let mut pending = vec![field];
    let mut seen = HashSet::new();
    let mut sites = Vec::new();
    while let Some(node) = pending.pop() {
        if !seen.insert(node) {
            continue;
        }
        stats.node_visits += 1;
        for usage in uses.of(node) {
            stats.node_visits += 1;
            let user = usage.user;
            if graph.op(user) == Opcode::VariantWitness {
                continue;
            }
            if graph.op(user) == Opcode::Phi {
                if let UseKind::Operand(index) = usage.kind
                    && index > 0
                {
                    sites.push(graph.input(graph.input(user, 0), index - 1));
                }
            } else if is_anchored_value(graph.op(user)) {
                sites.push(graph.control(user));
            } else if is_control_flow(graph, user) {
                sites.push(user);
            } else {
                pending.push(user);
            }
        }
    }
    sites.sort_unstable();
    sites.dedup();
    stats.consumer_sites += sites.len();
    sites
}

/// Traversing the same proof exit refreshes evidence for a new dynamic value.
/// Between refreshes, crossing any source definition invalidates the old proof.
fn unchanged_until(
    cfg: &ControlFlow,
    cyclic: bool,
    proof: NodeId,
    at: NodeId,
    anchors: &[NodeId],
    stats: &mut WitnessStats,
) -> bool {
    if anchors.is_empty() || proof == at {
        return true;
    }
    if !cyclic {
        return true;
    }
    let mut pending: Vec<_> = cfg
        .successors(proof)
        .iter()
        .map(|&node| (node, false))
        .collect();
    let mut seen = HashSet::new();
    while let Some((node, dirty)) = pending.pop() {
        if node == proof || !seen.insert((node, dirty)) {
            continue;
        }
        stats.node_visits += 1;
        let dirty = dirty || anchors.contains(&node);
        if node == at && dirty {
            return false;
        }
        pending.extend(cfg.successors(node).iter().map(|&next| (next, dirty)));
    }
    true
}

pub fn check(
    graph: &FunctionGraph,
    cfg: &ControlFlow,
    dominators: &Dominators,
) -> Result<WitnessStats, (NodeId, &'static str)> {
    let stats = WitnessStats::default();
    let witnesses: Vec<_> = graph
        .ids()
        .filter(|&node| graph.op(node) == Opcode::VariantWitness)
        .collect();
    if witnesses.is_empty() {
        return Ok(stats);
    }
    let uses = Uses::new(graph);
    let mut facts = PredicateFacts::new(graph, cfg, dominators);
    // Include the use-index construction in measurable analysis work.
    facts.stats.node_visits += graph
        .ids()
        .map(|node| 1 + graph.inputs(node).len())
        .sum::<usize>();
    for node in witnesses {
        facts.stats.witnesses_checked += 1;
        let value = graph.resolve(graph.input(node, 0));
        let proof = graph.input(node, 1);
        let scope = graph.control(node);
        let variant = graph.payload(node);
        if !dominators.dominates(cfg, proof, scope) {
            return Err((node, "variant proof does not dominate its witness scope"));
        }
        let constructor = graph.op(value) == Opcode::MakeVariant && graph.payload(value) == variant;
        let scalar_evidence = facts
            .predicate_at(proof, scope)
            .and_then(|fact| predicate_variant(graph, value, fact))
            == Some(variant);
        if !constructor && variant_on_edge(graph, value, proof) != Some(variant) && !scalar_evidence
        {
            return Err((
                node,
                "VariantWitness lacks evidence for this exact enum value and variant",
            ));
        }
        if !facts.same_instance(value, proof, scope) {
            return Err((
                node,
                "variant witness source is unavailable or has changed since its proof",
            ));
        }
        for usage in uses.of(node) {
            let field = usage.user;
            if usage.kind != UseKind::Operand(1) || graph.op(field) != Opcode::VariantField {
                return Err((node, "witness used outside a variant field proof"));
            }
            for at in execution_sites(graph, &uses, field, &mut facts.stats) {
                if !dominators.dominates(cfg, scope, at) {
                    return Err((field, "variant field use escapes its witness scope"));
                }
                if !facts.same_instance(value, proof, at)
                    || (scalar_evidence && facts.predicate_at(proof, at).is_none())
                {
                    return Err((
                        field,
                        "variant witness refers to a previous dynamic value instance",
                    ));
                }
            }
        }
    }
    Ok(facts.stats)
}

#[cfg(test)]
#[path = "refinement_tests.rs"]
mod tests;
