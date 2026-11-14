//! Scalar Phi-translation PRE with explicit pre-merge execution evidence.
//!
//! Floating node existence is not availability. An existing expression must
//! reach a live execution site dominating its predecessor. Only scalar values
//! are translated; effects, memory, scopes and the control graph stay intact.

mod expression;
mod plan;
mod proof;

use crate::analysis::{cfg::ControlFlow, dominators::Dominators, loops::LoopInfo, uses::Uses};
use crate::{FunctionGraph, GraphLevel, NodeId};
use kfrc_body::fx_hash::HashMap;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct PreStats {
    pub expressions_replaced: usize,
    pub expressions_inserted: usize,
    pub phis_created: usize,
    pub analysis_visits: usize,
    pub budget_bailouts: usize,
    pub dirty_nodes: Vec<NodeId>,
}

#[derive(Clone, Copy)]
struct Limits {
    plans: usize,
    nodes: usize,
    analysis: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            plans: 32,
            nodes: 64,
            analysis: 32_768,
        }
    }
}

struct Work {
    visits: usize,
    remaining: usize,
    exhausted: bool,
    limited: usize,
}

impl Work {
    fn charge(&mut self, amount: usize) -> Option<()> {
        let paid = amount.min(self.remaining);
        self.visits += paid;
        self.remaining -= paid;
        if paid != amount {
            self.exhausted = true;
            None
        } else {
            Some(())
        }
    }
}

pub(crate) fn run(graph: &mut FunctionGraph) -> PreStats {
    run_with_limits(graph, Limits::default())
}

fn run_with_limits(graph: &mut FunctionGraph, limits: Limits) -> PreStats {
    let mut stats = PreStats::default();
    if graph.level != GraphLevel::Specialized {
        return stats;
    }
    graph.resolve_edges();
    let mut work = Work {
        visits: 0,
        remaining: limits.analysis,
        exhausted: false,
        limited: 0,
    };
    let mut nodes = Vec::new();
    let mut candidates = Vec::new();
    for node in graph.ids() {
        if work.charge(1 + graph.inputs(node).len()).is_none() {
            stats.analysis_visits = work.visits;
            stats.budget_bailouts = 1;
            return stats;
        }
        nodes.push(node);
        if expression::eligible(graph, node)
            && graph
                .inputs(node)
                .iter()
                .any(|&input| graph.op(graph.resolve(input)) == crate::Opcode::Phi)
        {
            candidates.push(node);
        }
    }
    if candidates.is_empty() {
        stats.analysis_visits = work.visits;
        return stats;
    }
    if work.charge(nodes.len() * 3).is_none() {
        stats.analysis_visits = work.visits;
        stats.budget_bailouts = 1;
        return stats;
    }
    let cfg = ControlFlow::new(graph);
    let dominators = Dominators::new(&cfg);
    let loops = LoopInfo::new(&cfg, &dominators);
    let loop_work = loops.stats();
    if work
        .charge(loop_work.member_edge_visits + loop_work.scc_edge_visits)
        .is_none()
    {
        stats.analysis_visits = work.visits;
        stats.budget_bailouts = 1;
        return stats;
    }
    let uses = Uses::new(graph);
    let Some(live) = proof::live_nodes(graph, &cfg, &nodes, &mut work) else {
        stats.analysis_visits = work.visits;
        stats.budget_bailouts = 1;
        return stats;
    };
    let mut expressions: HashMap<expression::Expression, Vec<NodeId>> = HashMap::default();
    for &node in &nodes {
        if live[node.index()] && expression::eligible(graph, node) {
            expressions
                .entry(expression::Expression::of(graph, node, graph.inputs(node)))
                .or_default()
                .push(node);
        }
    }
    let mut proof = proof::Proof::new(graph, &cfg, &dominators, &loops, &uses, &live, &mut work);
    let mut plans = Vec::new();
    let mut growth = 0;
    for candidate in candidates.into_iter().filter(|node| live[node.index()]) {
        if plans.len() == limits.plans {
            stats.budget_bailouts += 1;
            break;
        }
        if let Some(plan) = plan::Plan::new(graph, candidate, &expressions, &mut proof) {
            let cost = plan.cost();
            if cost > limits.nodes.saturating_sub(growth) {
                stats.budget_bailouts += 1;
                continue;
            }
            growth += cost;
            plans.push(plan);
        }
        if proof.work.exhausted {
            break;
        }
    }
    drop(proof);
    stats.analysis_visits = work.visits;
    stats.budget_bailouts += work.limited;
    if work.exhausted {
        // Analysis exhaustion aborts the whole batch; no partially proved
        // transformation and no prefix of this batch is published.
        stats.budget_bailouts += 1;
        return stats;
    }
    for plan in plans {
        plan.apply(graph, &uses, &mut stats);
    }
    if stats.expressions_replaced != 0 {
        graph.resolve_edges();
        stats.dirty_nodes.retain(|node| !graph.is_dead(*node));
        stats.dirty_nodes.sort_unstable();
        stats.dirty_nodes.dedup();
    }
    stats
}

#[cfg(test)]
mod tests;
