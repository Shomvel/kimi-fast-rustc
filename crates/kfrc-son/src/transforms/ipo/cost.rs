//! Cost policy and per-call explanations. Legality and growth budgets remain
//! in the inliner; a cheap estimate never grants permission to splice a call.
use kfrc_body::fx_hash::HashMap;

use crate::analysis::cost::{CostModel, GraphCost, estimate_graph};
use crate::{FunctionGraph, NodeId};

use super::{GraphProvider, InlineBudget, InlineSkip};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum InlineCostPolicy {
    /// Retain the accepted node-budget policy without computing estimates.
    #[default]
    Legacy,
    /// Record costs and outcomes without changing ordering or decisions.
    Observe,
    /// Relative code-size units from the selected model, not IR node counts.
    /// Unknown estimates retain the call. Hard depth/growth limits still apply.
    CodeSize { max_units: u64, free_units: u64 },
}

#[derive(Clone, Debug)]
pub struct InlineDecision {
    pub call: NodeId,
    pub symbol: String,
    pub depth: usize,
    pub source_nodes: usize,
    pub budget: InlineBudget,
    pub nodes_added_before: usize,
    pub costed_calls_before: usize,
    pub model: &'static str,
    pub policy: InlineCostPolicy,
    /// None means missing source or the bounded analysis budget was exhausted.
    pub estimate: Option<GraphCost>,
    pub sole_call_site: bool,
    pub free: bool,
    /// None means inlined; a rejection keeps the precise existing legality or
    /// budget reason, rather than relabeling all failures as excessive cost.
    pub rejection: Option<InlineSkip>,
}

pub(super) struct Costs<'a> {
    pub policy: InlineCostPolicy,
    model: &'a dyn CostModel,
    cache: HashMap<String, Option<GraphCost>>,
    remaining_nodes: usize,
    pub nodes_visited: usize,
}

impl<'a> Costs<'a> {
    pub fn new(policy: InlineCostPolicy, model: &'a dyn CostModel, max_nodes: usize) -> Self {
        Self {
            policy,
            model,
            cache: HashMap::default(),
            remaining_nodes: max_nodes,
            nodes_visited: 0,
        }
    }

    pub fn estimate(
        &mut self,
        symbol: &str,
        provider: &(impl GraphProvider + ?Sized),
    ) -> Option<GraphCost> {
        if self.policy == InlineCostPolicy::Legacy {
            return None;
        }
        if let Some(cost) = self.cache.get(symbol) {
            return *cost;
        }
        let cost = provider.graph(symbol).and_then(|source| {
            if source.live_count() > self.remaining_nodes {
                return None;
            }
            self.remaining_nodes -= source.live_count();
            self.nodes_visited += source.live_count();
            Some(estimate_graph(source, self.model))
        });
        self.cache.insert(symbol.into(), cost);
        cost
    }

    pub fn priority(
        &mut self,
        graph: &FunctionGraph,
        call: NodeId,
        provider: &(impl GraphProvider + ?Sized),
    ) -> u64 {
        let symbol = &graph.callees[graph.payload(call) as usize].symbol;
        if matches!(self.policy, InlineCostPolicy::CodeSize { .. }) {
            self.estimate(symbol, provider)
                .and_then(|cost| cost.code_size.total())
                .unwrap_or(u64::MAX)
        } else {
            provider.graph(symbol).map_or(0, FunctionGraph::live_count) as u64
        }
    }

    pub fn model_name(&self) -> &'static str {
        self.model.name()
    }
}
