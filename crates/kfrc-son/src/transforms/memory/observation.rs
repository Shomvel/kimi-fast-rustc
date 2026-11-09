//! Remove observations whose full Rust value is already known to be defined.
//!
//! Facts are tied to execution controls and exact SSA identities. Phi inputs
//! are proved on their own predecessor edges; loop-carried Phis never prove
//! themselves. Calls, assumptions, and the first poison observation stay fixed.
use crate::analysis::cfg::ControlFlow;
use crate::analysis::dominators::Dominators;
use crate::analysis::uses::{UseKind, Uses};
use crate::{FunctionGraph, GraphLevel, NodeId, Opcode, ValueType};
use kfrc_body::TypeShape;
use kfrc_body::fx_hash::{HashMap, HashSet};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ObservationStats {
    pub observations_removed: usize,
    /// Node inventory, Uses and CFG each index N nodes; dominance indexes C controls.
    pub nodes_indexed: usize,
    /// Definition-proof queries and dynamic-scope dependency visits.
    pub node_visits: usize,
    pub dirty_nodes: Vec<NodeId>,
}

pub fn run(graph: &mut FunctionGraph) -> ObservationStats {
    let mut stats = ObservationStats::default();
    // S entry and call result observation contracts do not describe R or L.
    if graph.level != GraphLevel::Specialized {
        return stats;
    }
    graph.resolve_edges();
    let nodes: Vec<_> = graph.ids().collect();
    if !nodes.iter().any(|&n| graph.op(n) == Opcode::RequireDefined) {
        stats.nodes_indexed = nodes.len();
        return stats;
    }
    let uses = Uses::new(graph);
    let cfg = ControlFlow::new(graph);
    let dominators = Dominators::new(&cfg);
    stats.nodes_indexed = nodes.len() * 3 + cfg.order.len();
    let mut facts = vec![Vec::new(); graph.len()];
    for &source in &nodes {
        let op = graph.op(source);
        // Source return contracts apply independently of physical ABI transport.
        let observed_return = matches!(op, Opcode::Call | Opcode::CallIndirect);
        if op != Opcode::Start && op != Opcode::RequireDefined && !observed_return {
            continue;
        }
        let Some(continuation) = uses.of(source).iter().find_map(|usage| {
            (usage.kind == UseKind::Operand(0)
                && graph.op(usage.user) == Opcode::Projection
                && graph.payload(usage.user) == 0)
                .then_some(usage.user)
        }) else {
            continue;
        };
        if op == Opcode::RequireDefined {
            facts[graph.input(source, 1).index()].push(continuation);
        }
        for usage in uses.of(source) {
            if op != Opcode::RequireDefined
                && usage.kind == UseKind::Operand(0)
                && graph.op(usage.user) == Opcode::Projection
                && graph.payload(usage.user) >= 2
                && matches!(graph.value_type(usage.user), ValueType::Rust(_))
            {
                facts[usage.user.index()].push(continuation);
            }
        }
    }
    let mut proof = Proof {
        graph,
        cfg: &cfg,
        dominators: &dominators,
        facts: &facts,
        memo: HashMap::default(),
        active: HashSet::default(),
        scopes: HashMap::default(),
        visits: 0,
    };
    let mut removed = Vec::new();
    for node in nodes
        .into_iter()
        .filter(|&n| graph.op(n) == Opcode::RequireDefined)
    {
        if uses.of(node).iter().any(|usage| {
            usage.kind != UseKind::Operand(0)
                || graph.op(usage.user) != Opcode::Projection
                || graph.payload(usage.user) > 1
        }) {
            continue;
        }
        if proof.known(graph.input(node, 1), graph.control(node), &mut 512) {
            removed.push(node);
        }
    }
    stats.node_visits = proof.visits;
    // Finish every proof against the unchanged snapshot. A removed fact is
    // justified by an earlier dominating fact, never by its own continuation.
    for node in removed {
        let replacements = [graph.control(node), graph.input(node, 0)];
        for usage in uses.of(node) {
            let projection = usage.user;
            stats
                .dirty_nodes
                .extend(uses.of(projection).iter().map(|usage| usage.user));
            graph.replace(projection, replacements[graph.payload(projection) as usize]);
        }
        graph.remove_node(node);
        stats.observations_removed += 1;
    }
    graph.resolve_edges();
    stats.dirty_nodes.retain(|&node| !graph.is_dead(node));
    stats.dirty_nodes.sort_unstable();
    stats.dirty_nodes.dedup();
    stats
}

struct Proof<'a> {
    graph: &'a FunctionGraph,
    cfg: &'a ControlFlow,
    dominators: &'a Dominators,
    facts: &'a [Vec<NodeId>],
    memo: HashMap<(NodeId, NodeId), bool>,
    active: HashSet<(NodeId, NodeId)>,
    scopes: HashMap<NodeId, Option<Vec<NodeId>>>,
    visits: usize,
}
impl Proof<'_> {
    fn dominates(&self, before: NodeId, after: NodeId) -> bool {
        self.dominators.dominates(self.cfg, before, after)
    }

    fn known(&mut self, value: NodeId, at: NodeId, budget: &mut usize) -> bool {
        self.visits += 1;
        if let Some(&known) = self.memo.get(&(value, at)) {
            return known;
        }
        if *budget == 0 || !self.active.insert((value, at)) {
            return false;
        }
        *budget -= 1;
        let known = self.prove(value, at, budget);
        self.active.remove(&(value, at));
        self.memo.insert((value, at), known);
        known
    }

    fn prove(&mut self, value: NodeId, at: NodeId, budget: &mut usize) -> bool {
        for index in 0..self.facts[value.index()].len() {
            let fact = self.facts[value.index()][index];
            if self.dominates(fact, at) && self.same_instance(value, fact) {
                return true;
            }
        }
        let graph = self.graph;
        match graph.op(value) {
            Opcode::Const => true,
            Opcode::Aggregate | Opcode::MakeVariant | Opcode::WidePtr => graph
                .inputs(value)
                .iter()
                .all(|&input| self.known(input, at, budget)),
            Opcode::Repeat => {
                let empty = graph.rust_type(value).is_some_and(|ty| {
                    matches!(
                        graph.oracle.semantic().shape(ty),
                        TypeShape::Array { length: 0, .. }
                    )
                });
                empty || self.known(graph.input(value, 0), at, budget)
            }
            Opcode::Field => self.known(graph.input(value, 0), at, budget),
            Opcode::VariantField => {
                let source = graph.input(value, 0);
                let (variant, _) = graph.variant_fields[graph.payload(value) as usize];
                let constructor =
                    graph.op(source) == Opcode::MakeVariant && graph.payload(source) == variant;
                let witnessed = graph.inputs(value).get(1).is_some_and(|&witness| {
                    graph.op(witness) == Opcode::VariantWitness
                        && graph.input(witness, 0) == source
                        && graph.payload(witness) == variant
                        && self.dominates(graph.control(witness), at)
                });
                // The tag witness does not certify payload initialization.
                (constructor || witnessed) && self.known(source, at, budget)
            }
            Opcode::Phi => {
                let region = graph.input(value, 0);
                let Some(&position) = self.cfg.position.get(&region) else {
                    return false;
                };
                if !self.dominates(region, at)
                    || self.cfg.predecessors[position]
                        .iter()
                        .any(|&pred| pred >= position)
                {
                    return false;
                }
                graph.inputs(value)[1..]
                    .iter()
                    .zip(graph.inputs(region))
                    .all(|(&incoming, &edge)| self.known(incoming, edge, budget))
            }
            _ => false,
        }
    }

    /// A static value name can denote a new dynamic value on another loop
    /// iteration. Every Phi/fixed leaf must have executed before the proof;
    /// together with proof dominance, no redefinition can bypass that proof.
    fn same_instance(&mut self, value: NodeId, fact: NodeId) -> bool {
        if !self.scopes.contains_key(&value) {
            let mut pending = vec![value];
            let mut seen = HashSet::default();
            let mut anchors = Vec::new();
            let mut complete = true;
            while let Some(node) = pending.pop() {
                if !seen.insert(node) {
                    continue;
                }
                self.visits += 1;
                if seen.len() > 512 {
                    complete = false;
                    break;
                }
                match self.graph.op(node) {
                    Opcode::Phi => anchors.push(self.graph.input(node, 0)),
                    Opcode::VariantWitness => anchors.push(self.graph.control(node)),
                    Opcode::Projection => anchors.push(self.graph.input(node, 0)),
                    op if op.is_fixed() => anchors.push(self.graph.control(node)),
                    _ => pending.extend_from_slice(self.graph.inputs(node)),
                }
            }
            anchors.sort_unstable();
            anchors.dedup();
            self.scopes.insert(value, complete.then_some(anchors));
        }
        self.scopes[&value]
            .as_ref()
            .is_some_and(|anchors| anchors.iter().all(|&anchor| self.dominates(anchor, fact)))
    }
}

#[cfg(test)]
#[path = "observation_tests.rs"]
mod tests;
