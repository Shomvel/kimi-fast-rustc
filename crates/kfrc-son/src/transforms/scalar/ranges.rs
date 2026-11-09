//! Fold comparisons only when both operands have non-poison range proofs.
use crate::{FunctionGraph, Opcode, analysis::ranges::RangeFacts};
use std::collections::HashSet;

#[derive(Clone, Debug, Default)]
pub struct RangeFoldStats {
    pub comparisons_folded: usize,
    pub fact_queries: usize,
    pub node_visits: usize,
    pub dirty_nodes: Vec<crate::NodeId>,
}

pub fn run(graph: &mut FunctionGraph) -> RangeFoldStats {
    graph.resolve_edges();
    // A scalar predicate can be compiler evidence for a VariantWitness. Its
    // truth is insufficient to replace the proof syntax until the witness
    // checker also understands type-validity evidence.
    let protected: HashSet<_> = graph
        .ids()
        .filter(|&node| graph.op(node) == Opcode::VariantWitness)
        .filter_map(|node| {
            let proof = graph.resolve(graph.input(node, 1));
            if graph.op(proof) != Opcode::Projection {
                return None;
            }
            let source = graph.resolve(graph.input(proof, 0));
            let condition = match graph.op(source) {
                Opcode::If => graph.input(source, 0),
                Opcode::Assume => graph.input(source, 1),
                _ => return None,
            };
            crate::analysis::refinement::canonical_condition(graph, condition).map(|(base, _)| base)
        })
        .collect();
    let mut facts = RangeFacts::new(graph);
    let replacements: Vec<_> = graph
        .ids()
        .filter(|&node| graph.op(node) == Opcode::Compare && !protected.contains(&node))
        .filter_map(|node| facts.compare(node).map(|value| (node, value)))
        .collect();
    let mut stats = RangeFoldStats {
        comparisons_folded: replacements.len(),
        fact_queries: facts.queries,
        node_visits: facts.node_visits,
        dirty_nodes: vec![],
    };
    drop(facts);
    if replacements.is_empty() {
        return stats;
    }
    let uses = crate::analysis::uses::Uses::new(graph);
    for (node, value) in replacements {
        let ty = graph.rust_type(node).expect("Compare returns bool");
        let replacement = graph.add_constant(u128::from(value), ty);
        stats
            .dirty_nodes
            .extend(uses.of(node).iter().map(|usage| usage.user));
        graph.replace(node, replacement);
    }
    stats.dirty_nodes.sort_unstable();
    stats.dirty_nodes.dedup();
    graph.resolve_edges();
    stats
}
