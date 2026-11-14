//! Give a reducible, single-latch loop one dedicated entry edge.
//!
//! Coalesce only outside predecessors of the header, preserving their value
//! and effect Phi slots together. Exits and backedges are not redirected.
//! This is a bounded subset of LoopSimplify, not an LCSSA or rotation pass.

use crate::analysis::{cfg::ControlFlow, dominators::Dominators, loops::LoopInfo};
use crate::transforms::utils::cfg::{CfgEditError, CfgEditor};
use crate::{FunctionGraph, GraphLevel, NodeId, Opcode};
use kfrc_body::fx_hash::HashMap;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct LoopSimplifyStats {
    pub preheaders_created: usize,
    pub phis_created: usize,
    pub nodes_added: usize,
    pub analysis_visits: usize,
    pub budget_bailouts: usize,
}

#[derive(Clone, Copy)]
struct Limits {
    analysis: usize,
    preheaders: usize,
    nodes: usize,
    phis: usize,
    entries: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            analysis: 32_768,
            preheaders: 4,
            nodes: 128,
            phis: 30,
            entries: 8,
        }
    }
}

fn charge(stats: &mut LoopSimplifyStats, limits: Limits, visits: usize) -> bool {
    let available = limits.analysis.saturating_sub(stats.analysis_visits);
    stats.analysis_visits += visits.min(available);
    if visits > available {
        stats.budget_bailouts += 1;
        false
    } else {
        true
    }
}

pub(crate) fn run(graph: &mut FunctionGraph) -> Result<LoopSimplifyStats, CfgEditError> {
    run_with_limits(graph, Limits::default())
}

fn run_with_limits(
    graph: &mut FunctionGraph,
    limits: Limits,
) -> Result<LoopSimplifyStats, CfgEditError> {
    let mut stats = LoopSimplifyStats::default();
    if graph.level != GraphLevel::Specialized {
        return Ok(stats);
    }
    graph.resolve_edges();
    let mut potential = false;
    let mut phis: HashMap<NodeId, Vec<NodeId>> = HashMap::default();
    for node in graph.ids() {
        if !charge(&mut stats, limits, 1 + graph.inputs(node).len()) {
            return Ok(stats);
        }
        potential |= graph.op(node) == Opcode::Region && graph.inputs(node).len() > 2;
        if graph.op(node) == Opcode::Phi {
            phis.entry(graph.input(node, 0)).or_default().push(node);
        }
    }
    if !potential || !charge(&mut stats, limits, graph.len().saturating_mul(3)) {
        return Ok(stats);
    }
    let cfg = ControlFlow::new(graph);
    let dominators = Dominators::new(&cfg);
    let loops = LoopInfo::new(&cfg, &dominators);
    let loop_stats = loops.stats();
    if !charge(
        &mut stats,
        limits,
        loop_stats.member_edge_visits + loop_stats.scc_edge_visits,
    ) {
        return Ok(stats);
    }
    let mut plans = Vec::new();
    let mut growth = 0usize;
    for loop_ in loops.loops() {
        if !charge(
            &mut stats,
            limits,
            loop_.members.len() + loop_.entry_edges.len(),
        ) {
            return Ok(stats);
        }
        let header = loop_.header;
        if loop_.contains_irreducible
            || loop_.parent.is_some()
            || loop_.latches.len() != 1
            || loop_.entry_edges.len() < 2
            || graph.op(header) != Opcode::Region
            || loop_.entry_edges.iter().any(|edge| edge.to != header)
        {
            continue;
        }
        let predecessors = graph.inputs(header);
        if !charge(&mut stats, limits, predecessors.len()) {
            return Ok(stats);
        }
        if predecessors
            .iter()
            .any(|&pred| graph.op(pred) != Opcode::Jump || cfg.successors(pred) != [header])
        {
            continue;
        }
        let group: Vec<_> = predecessors
            .iter()
            .map(|&pred| !loop_.contains(pred))
            .collect();
        let entries = group.iter().filter(|&&selected| selected).count();
        if entries != loop_.entry_edges.len() || entries + 1 != predecessors.len() {
            continue;
        }
        let header_phis = phis.get(&header).map_or(&[][..], Vec::as_slice);
        for &phi in header_phis {
            if !charge(&mut stats, limits, graph.inputs(phi).len()) {
                return Ok(stats);
            }
            if graph.inputs(phi).len() != predecessors.len() + 1 {
                return Err(CfgEditError::MalformedPhi {
                    region: header,
                    phi,
                });
            }
        }
        let cost = header_phis.len() + 2;
        if entries > limits.entries
            || header_phis.len() > limits.phis
            || plans.len() == limits.preheaders
            || cost > limits.nodes.saturating_sub(growth)
        {
            stats.budget_bailouts += 1;
            continue;
        }
        growth += cost;
        plans.push((header, group));
    }
    // Account for the editor's Phi inventory before applying any plan. Exhaustion
    // discards the whole batch, so a later query never observes a partial edit.
    if !charge(
        &mut stats,
        limits,
        plans
            .len()
            .saturating_mul(graph.len().saturating_add(growth)),
    ) {
        return Ok(stats);
    }
    let before = graph.len();
    for (header, group) in plans {
        if let Some(grouped) = CfgEditor::new(graph).group_predecessors(header, &group)? {
            stats.preheaders_created += 1;
            stats.phis_created += grouped.phis_created;
        }
    }
    stats.nodes_added = graph.len() - before;
    graph.resolve_edges();
    Ok(stats)
}

#[cfg(test)]
#[path = "loop_simplify_tests.rs"]
mod tests;
