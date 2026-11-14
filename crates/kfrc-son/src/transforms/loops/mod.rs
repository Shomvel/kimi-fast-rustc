//! Bounded complete unrolling and modular integer strength reduction in S.
//!
//! Plans describe disjoint, non-nested loops in one immutable snapshot. All
//! shape, arithmetic and growth checks finish before any clone is published.
//! NodeIds remain stable: the pipeline owns scalar cleanup and compaction.

mod clone;
mod count;
mod plan;
pub(crate) mod strength_reduce;
mod values;

use crate::analysis::{cfg::ControlFlow, dominators::Dominators, loops::LoopInfo, uses::Uses};
use crate::transforms::{control::simplify, utils::cfg::CfgEditError};
use crate::{FunctionGraph, GraphLevel, NodeId, Opcode};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct UnrollStats {
    pub loops_unrolled: usize,
    pub iterations_cloned: usize,
    /// Every newly allocated graph node, including induction constants and
    /// fixed-operation projections. Reused header/control aliases cost zero.
    pub nodes_cloned: usize,
    pub analysis_visits: usize,
    pub budget_bailouts: usize,
    pub dirty_nodes: Vec<NodeId>,
    pub exposed_slots: Vec<NodeId>,
}

#[derive(Clone, Copy)]
struct Limits {
    iterations: usize,
    template_nodes: usize,
    loop_nodes: usize,
    total_nodes: usize,
    loops: usize,
    analysis: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            iterations: 8,
            template_nodes: 64,
            loop_nodes: 256,
            total_nodes: 512,
            loops: 4,
            analysis: 32_768,
        }
    }
}

struct Work {
    visits: usize,
    remaining: usize,
    exhausted: bool,
    limits_hit: usize,
}

impl Work {
    fn charge(&mut self, count: usize) -> Option<()> {
        let paid = count.min(self.remaining);
        self.visits += paid;
        self.remaining -= paid;
        if paid != count {
            self.exhausted = true;
            None
        } else {
            Some(())
        }
    }
}

pub(crate) fn run(graph: &mut FunctionGraph) -> Result<UnrollStats, CfgEditError> {
    run_with_limits(graph, Limits::default())
}

fn run_with_limits(graph: &mut FunctionGraph, limits: Limits) -> Result<UnrollStats, CfgEditError> {
    let mut stats = UnrollStats::default();
    if graph.level != GraphLevel::Specialized {
        return Ok(stats);
    }
    graph.resolve_edges();
    let mut work = Work {
        visits: 0,
        remaining: limits.analysis,
        exhausted: false,
        limits_hit: 0,
    };
    let mut nodes = Vec::new();
    let mut potential = false;
    for node in graph.ids() {
        if work.charge(1 + graph.inputs(node).len()).is_none() {
            stats.analysis_visits = work.visits;
            stats.budget_bailouts = 1;
            return Ok(stats);
        }
        nodes.push(node);
        potential |= graph.op(node) == Opcode::Phi && graph.inputs(node).len() == 3;
    }
    if !potential {
        stats.analysis_visits = work.visits;
        return Ok(stats);
    }
    // Charge the input inventories before building the immutable indices.
    if work.charge(nodes.len() * 3).is_none() {
        stats.analysis_visits = work.visits;
        stats.budget_bailouts = 1;
        return Ok(stats);
    }
    let cfg = ControlFlow::new(graph);
    let dominators = Dominators::new(&cfg);
    let loops = LoopInfo::new(&cfg, &dominators);
    let uses = Uses::new(graph);
    let facts = loops.stats();
    if work
        .charge(facts.member_edge_visits + facts.scc_edge_visits)
        .is_none()
    {
        stats.analysis_visits = work.visits;
        stats.budget_bailouts = 1;
        return Ok(stats);
    }
    let mut plans = Vec::new();
    let mut growth = 0;
    for loop_ in loops.loops() {
        if plans.len() == limits.loops {
            stats.budget_bailouts += 1;
            break;
        }
        if let Some(plan) = plan::Plan::new(
            graph,
            &cfg,
            &dominators,
            &uses,
            &nodes,
            loop_,
            limits,
            &mut work,
        ) {
            let cost = plan.cost();
            if cost > limits.loop_nodes || cost > limits.total_nodes.saturating_sub(growth) {
                stats.budget_bailouts += 1;
                continue;
            }
            growth += cost;
            plans.push(plan);
        }
        if work.exhausted {
            stats.budget_bailouts += 1;
            break;
        }
    }
    stats.analysis_visits = work.visits;
    stats.budget_bailouts += work.limits_hit;
    // The accepted plans have disjoint control members and no nested loops.
    // Their NodeIds and templates stay intact until all plans are applied.
    for plan in plans {
        plan.apply(graph, &mut stats)?;
    }
    if stats.loops_unrolled != 0 {
        simplify::run(graph)?;
        graph.resolve_edges();
        stats.dirty_nodes.retain(|node| !graph.is_dead(*node));
        stats.dirty_nodes.sort_unstable();
        stats.dirty_nodes.dedup();
        stats.exposed_slots.retain(|node| !graph.is_dead(*node));
        stats.exposed_slots.sort_unstable();
        stats.exposed_slots.dedup();
    }
    Ok(stats)
}

#[cfg(test)]
mod tests;
