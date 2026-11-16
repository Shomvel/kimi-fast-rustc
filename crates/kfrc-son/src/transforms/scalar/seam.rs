//! One shared worklist for changes exposed by graph splicing.
//!
//! A worklist can span several splices. Its use index is built once, then
//! extended with new nodes and repaired at changed seams. CFG editors must
//! discard it because they can change arbitrary existing operand lists.
mod fields;

use super::{aggregate, instcombine};
use crate::{FunctionGraph, NodeId, Opcode};

// One local rule can allocate a constant plus its replacement operation.
// Rules must retain this bound; the runtime assertion below checks the contract.
const MAX_REWRITE_NODES: usize = 2;

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct SeamStats {
    pub nodes_indexed: usize,
    pub nodes_visited: usize,
    pub fields_folded: usize,
    pub variants_folded: usize,
    pub aggregates_simplified: usize,
    pub constants_folded: usize,
    pub identities_simplified: usize,
    pub control_dirty: bool,
    pub allocation_limited: bool,
}

pub(crate) fn run_seeded(graph: &mut FunctionGraph, seeds: &[NodeId]) -> SeamStats {
    if seeds.is_empty() {
        return SeamStats::default();
    }
    Worklist::new(graph).run(graph, seeds)
}

pub(crate) struct Worklist {
    users: Vec<Vec<NodeId>>,
    inputs: Vec<Vec<NodeId>>,
    fields: fields::Fields,
    queued: Vec<bool>,
    initial_indexed: usize,
}

impl Worklist {
    pub(crate) fn new(graph: &FunctionGraph) -> Self {
        let mut work = Self {
            users: vec![Vec::new(); graph.len()],
            inputs: vec![Vec::new(); graph.len()],
            fields: fields::Fields::default(),
            queued: vec![false; graph.len()],
            initial_indexed: 0,
        };
        let mut stats = SeamStats::default();
        for node in graph.ids() {
            work.index(graph, node, &mut stats);
        }
        work.initial_indexed = stats.nodes_indexed;
        work
    }

    // Old reverse edges may remain after an operand edit. They only enqueue
    // an extra candidate: its current inputs and expression are always read
    // from the graph. Never use this conservative index as a use-count proof.
    fn index(&mut self, graph: &FunctionGraph, node: NodeId, stats: &mut SeamStats) {
        if graph.is_dead(node) {
            return;
        }
        stats.nodes_indexed += 1;
        self.fields.record(graph, node);
        let inputs: Vec<_> = graph
            .inputs(node)
            .iter()
            .map(|&input| graph.resolve(input))
            .filter(|input| !input.is_none())
            .collect();
        if self.inputs[node.index()] != inputs {
            for &input in &inputs {
                self.users[input.index()].push(node);
            }
            self.inputs[node.index()] = inputs;
        }
    }

    /// Existing changed operand users are seeds. Appended nodes are indexed
    /// and seeded automatically, so all substituted parameter uses participate.
    pub(crate) fn run(&mut self, graph: &mut FunctionGraph, seeds: &[NodeId]) -> SeamStats {
        self.run_bounded(graph, seeds, usize::MAX)
    }

    pub(crate) fn run_bounded(
        &mut self,
        graph: &mut FunctionGraph,
        seeds: &[NodeId],
        max_new_nodes: usize,
    ) -> SeamStats {
        let maximum_len = graph.len().saturating_add(max_new_nodes);
        let mut stats = SeamStats {
            nodes_indexed: std::mem::take(&mut self.initial_indexed),
            ..SeamStats::default()
        };
        let first_new = self.users.len();
        self.users.resize_with(graph.len(), Vec::new);
        self.inputs.resize_with(graph.len(), Vec::new);
        for index in first_new..graph.len() {
            self.index(graph, NodeId(index as u32), &mut stats);
        }
        let mut pending: Vec<_> = seeds
            .iter()
            .copied()
            .chain((first_new..graph.len()).map(|index| NodeId(index as u32)))
            .map(|node| graph.resolve(node))
            .filter(|node| !node.is_none() && !graph.is_dead(*node))
            .collect();
        pending.sort_unstable();
        pending.dedup();
        for &node in &pending {
            if node.index() < first_new {
                self.index(graph, node, &mut stats);
            }
        }
        pending.reverse();
        self.queued.resize(graph.len(), false);
        for node in &pending {
            self.queued[node.index()] = true;
        }
        let fuel = graph.len().saturating_mul(8).saturating_add(seeds.len());
        while let Some(node) = pending.pop() {
            self.queued[node.index()] = false;
            if stats.nodes_visited >= fuel {
                for &node in &pending {
                    self.queued[node.index()] = false;
                }
                break;
            }
            if graph.is_dead(node) {
                continue;
            }
            if matches!(graph.op(node), Opcode::If | Opcode::VariantSwitch) {
                let condition = graph.resolve(graph.input(node, 0));
                stats.control_dirty |=
                    matches!(graph.op(condition), Opcode::Const | Opcode::MakeVariant);
            }
            if !graph.control(node).is_none() {
                continue;
            }
            stats.nodes_visited += 1;
            if maximum_len.saturating_sub(graph.len()) < MAX_REWRITE_NODES {
                // The outer pipeline can finish these rewrites after the
                // inliner's nonrefundable allocation account has closed.
                stats.allocation_limited = true;
                continue;
            }
            let old_len = graph.len();
            let (replacement, kind) = if let Some(replacement) = aggregate::fold(graph, node) {
                (replacement, 0)
            } else if let Some((replacement, constant)) = instcombine::rewrite(graph, node) {
                (replacement, if constant { 1 } else { 2 })
            } else if let Some(replacement) = self.fields.replacement(graph, node) {
                (replacement, 0)
            } else {
                continue;
            };
            assert!(
                graph.len() - old_len <= MAX_REWRITE_NODES,
                "scalar seam rule exceeded its allocation contract"
            );
            if replacement == node || graph.value_type(replacement) != graph.value_type(node) {
                continue;
            }
            match kind {
                1 => stats.constants_folded += 1,
                2 => stats.identities_simplified += 1,
                _ => match graph.op(node) {
                    Opcode::Field => stats.fields_folded += 1,
                    Opcode::InsertField | Opcode::PointerCast | Opcode::WidePtr => {
                        stats.aggregates_simplified += 1
                    }
                    _ => stats.variants_folded += 1,
                },
            }
            graph.replace(node, replacement);
            self.users.resize_with(graph.len(), Vec::new);
            self.inputs.resize_with(graph.len(), Vec::new);
            self.queued.resize(graph.len(), false);
            for index in old_len..graph.len() {
                let created = NodeId(index as u32);
                self.index(graph, created, &mut stats);
                if !self.queued[index] {
                    self.queued[index] = true;
                    pending.push(created);
                }
            }
            for user in std::mem::take(&mut self.users[node.index()]) {
                self.index(graph, user, &mut stats);
                if !self.queued[user.index()] {
                    self.queued[user.index()] = true;
                    pending.push(user);
                }
            }
        }
        stats
    }
}
