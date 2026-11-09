//! One shared worklist for changes exposed by graph splicing.
//!
//! Constructing uses is one linear indexing pass. Rules only visit the given
//! seeds and users of replacements; unchanged callee interiors are not swept.
mod fields;

use super::{aggregate, instcombine};
use crate::{FunctionGraph, NodeId, Opcode};

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct SeamStats {
    pub nodes_indexed: usize,
    pub nodes_visited: usize,
    pub fields_folded: usize,
    pub variants_folded: usize,
    pub aggregates_simplified: usize,
    pub constants_folded: usize,
    pub identities_simplified: usize,
}

pub(crate) fn run_seeded(graph: &mut FunctionGraph, seeds: &[NodeId]) -> SeamStats {
    let mut stats = SeamStats::default();
    if seeds.is_empty() {
        return stats;
    }
    let mut users = vec![Vec::new(); graph.len()];
    let mut fields = fields::Fields::default();
    for node in graph.ids() {
        stats.nodes_indexed += 1;
        fields.record(graph, node);
        for &input in graph.inputs(node) {
            let input = graph.resolve(input);
            if !input.is_none() {
                users[input.index()].push(node);
            }
        }
    }
    let mut pending: Vec<_> = seeds
        .iter()
        .copied()
        .map(|node| graph.resolve(node))
        .filter(|node| !node.is_none() && !graph.is_dead(*node))
        .collect();
    pending.sort_unstable();
    pending.dedup();
    pending.reverse();
    let mut queued = vec![false; graph.len()];
    for node in &pending {
        queued[node.index()] = true;
    }
    let fuel = graph.len().saturating_mul(8).saturating_add(seeds.len());
    while let Some(node) = pending.pop() {
        queued[node.index()] = false;
        if stats.nodes_visited >= fuel {
            break;
        }
        if graph.is_dead(node) || !graph.control(node).is_none() {
            continue;
        }
        stats.nodes_visited += 1;
        let old_len = graph.len();
        let (replacement, kind) = if let Some(replacement) = aggregate::fold(graph, node) {
            (replacement, 0)
        } else if let Some((replacement, constant)) = instcombine::rewrite(graph, node) {
            (replacement, if constant { 1 } else { 2 })
        } else if let Some(replacement) = fields.replacement(graph, node) {
            (replacement, 0)
        } else {
            continue;
        };
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
        users.resize_with(graph.len(), Vec::new);
        queued.resize(graph.len(), false);
        for index in old_len..graph.len() {
            let created = NodeId(index as u32);
            for &input in graph.inputs(created) {
                let input = graph.resolve(input);
                if !input.is_none() {
                    users[input.index()].push(created);
                }
            }
            if !queued[index] {
                queued[index] = true;
                pending.push(created);
            }
        }
        for user in std::mem::take(&mut users[node.index()]) {
            users[replacement.index()].push(user);
            if !queued[user.index()] {
                queued[user.index()] = true;
                pending.push(user);
            }
        }
    }
    stats
}
