//! Local If rewrites justified by the same immutable observation proof.
use super::Proof;
use crate::analysis::known_bits;
use crate::{ComparePredicate as P, FunctionGraph, NodeId, Opcode};
use kfrc_body::fx_hash::HashSet;
use kfrc_body::{TyId, TypeShape};

pub(super) fn collect(proof: &mut Proof<'_>, nodes: &[NodeId]) -> Vec<(NodeId, TyId, bool)> {
    let graph = proof.graph;
    // A witness requires the original branch evidence even when that branch's
    // answer is known. Its Compare can also have unrelated, unprotected uses.
    let protected: HashSet<_> = nodes
        .iter()
        .copied()
        .filter(|&node| graph.op(node) == Opcode::VariantWitness)
        .filter_map(|node| {
            let edge = graph.input(node, 1);
            (graph.op(edge) == Opcode::Projection).then(|| graph.input(edge, 0))
        })
        .collect();
    let mut replacements = Vec::new();
    for &branch in nodes {
        if graph.op(branch) != Opcode::If || protected.contains(&branch) {
            continue;
        }
        let condition = graph.input(branch, 0);
        let Some(ty) = graph.rust_type(condition) else {
            continue;
        };
        if !matches!(graph.oracle.semantic().shape(ty), TypeShape::Bool) {
            continue;
        }
        let Some(answer) = compare(graph, condition) else {
            continue;
        };
        // Known bits describe only defined executions. The original observation
        // must already dominate this use of the same dynamic value instance.
        if proof.known(condition, graph.control(branch), &mut 512) {
            replacements.push((branch, ty, answer));
        }
    }
    replacements
}

fn compare(graph: &FunctionGraph, condition: NodeId) -> Option<bool> {
    if graph.op(condition) != Opcode::Compare {
        return None;
    }
    let predicate = P::from_payload(graph.payload(condition));
    if !matches!(
        predicate,
        P::UnsignedLess
            | P::UnsignedLessEqual
            | P::UnsignedGreater
            | P::UnsignedGreaterEqual
            | P::Equal
            | P::NotEqual
    ) {
        return None;
    }
    let a = known_bits::of(graph, graph.input(condition, 0))?.unsigned_bounds();
    let b = known_bits::of(graph, graph.input(condition, 1))?.unsigned_bounds();
    match predicate {
        P::UnsignedLess if a.1 < b.0 => Some(true),
        P::UnsignedLess if a.0 >= b.1 => Some(false),
        P::UnsignedLessEqual if a.1 <= b.0 => Some(true),
        P::UnsignedLessEqual if a.0 > b.1 => Some(false),
        P::UnsignedGreater if a.0 > b.1 => Some(true),
        P::UnsignedGreater if a.1 <= b.0 => Some(false),
        P::UnsignedGreaterEqual if a.0 >= b.1 => Some(true),
        P::UnsignedGreaterEqual if a.1 < b.0 => Some(false),
        P::Equal if a.1 < b.0 || b.1 < a.0 => Some(false),
        P::NotEqual if a.1 < b.0 || b.1 < a.0 => Some(true),
        _ => None,
    }
}
