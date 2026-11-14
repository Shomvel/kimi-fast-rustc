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
    /// Price the simplified source minus this call's ABI sequence. Nonpositive
    /// growth is free; other calls share a residual growth threshold.
    Residual { max_growth: u64 },
    /// Residual pricing with a larger threshold for cyclic emission sites.
    /// Ordinary/cold sites and context-independent preparation keep max_growth.
    /// Successful hot splices share total_hot_growth above the ordinary limit.
    CallSiteResidual {
        max_growth: u64,
        hot_growth: u64,
        total_hot_growth: u64,
    },
}

/// Static profitability hints, independent of memory/control legality.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InlineTemperature {
    Unknown,
    Ordinary,
    Hot,
    Cold,
}

impl InlineTemperature {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Ordinary => "ordinary",
            Self::Hot => "hot",
            Self::Cold => "cold",
        }
    }
}

impl InlineCostPolicy {
    pub(super) fn uses_size(self) -> bool {
        matches!(
            self,
            Self::CodeSize { .. } | Self::Residual { .. } | Self::CallSiteResidual { .. }
        )
    }
}

#[derive(Clone, Debug)]
pub struct InlineDecision {
    pub call: NodeId,
    pub symbol: String,
    pub depth: usize,
    pub source_nodes: usize,
    /// The selected call-site candidate, when constants refine the source.
    pub specialized_nodes: Option<usize>,
    pub budget: InlineBudget,
    pub nodes_added_before: usize,
    pub costed_calls_before: usize,
    pub model: &'static str,
    /// Effective policy after applying this site's static temperature.
    pub policy: InlineCostPolicy,
    pub temperature: InlineTemperature,
    /// Extra hot growth available before this decision; absent outside
    /// call-site emission. Only successful, non-exempt splices spend it.
    pub hot_growth_remaining: Option<u64>,
    pub hot_growth_charged: u64,
    /// None means missing source or the bounded analysis budget was exhausted.
    pub estimate: Option<GraphCost>,
    pub call_units: Option<u64>,
    pub sole_call_site: bool,
    pub free: bool,
    /// None means inlined; a rejection keeps the precise existing legality or
    /// budget reason, rather than relabeling all failures as excessive cost.
    pub rejection: Option<InlineSkip>,
}

/// Scratch graphs have their own identities; these records deliberately carry
/// symbols and depths rather than NodeIds from a discarded candidate.
#[derive(Clone, Debug)]
pub struct PreparationDecision {
    pub root: String,
    pub owner: String,
    pub symbol: String,
    pub depth: usize,
    pub source_nodes: usize,
    pub specialized_nodes: Option<usize>,
    pub action: &'static str,
    pub rejection: Option<InlineSkip>,
}

#[derive(Clone, Debug, Default)]
pub struct PreparationStats {
    pub calls_inlined: usize,
    pub costed_calls_inlined: usize,
    pub free_calls_inlined: usize,
    /// Prepared-source lookups, including cached unchanged/noreturn results.
    pub cache_hits: usize,
    /// Scratch splices selected from a positive cache entry; a subset of all
    /// splices that still pays the full allocation charge below.
    pub cached_calls_inlined: usize,
    pub cached_splice_nodes_allocated: usize,
    pub nodes_visited: usize,
    /// Bounded preparation-only control-producer queries, also included in
    /// nodes_visited. Exhausting this analysis account restores size ordering.
    pub priority_visit_budget: usize,
    pub priority_node_visits: usize,
    pub priority_budget_bailouts: usize,
    /// Existing field facts applied to new preparation nodes. Every allocated
    /// constant consumes the same nonrefundable clone/allocation account.
    pub fact_reapplication_visit_budget: usize,
    pub fact_reapplication_node_visits: usize,
    pub fact_reapplication_fields_replaced: usize,
    pub fact_reapplication_nodes_allocated: usize,
    pub fact_reapplication_budget_bailouts: usize,
    pub fact_node_visits: usize,
    pub clone_budget_used: usize,
    pub source_nodes_cloned: usize,
    pub transaction_nodes_cloned: usize,
    pub splice_nodes_cloned: usize,
    pub splice_nodes_allocated: usize,
    pub cleanup_runs: usize,
    pub cleanup_rounds: usize,
    pub cleanup_nodes_indexed: usize,
    pub cleanup_node_visits: usize,
    pub cleanup_nodes_added: usize,
    pub cleanup_branches_folded: usize,
    pub cleanup_observations_removed: usize,
    pub cleanup_conditions_folded: usize,
    pub maximum_depth: usize,
    pub budget_bailouts: usize,
    pub decisions_dropped: usize,
    pub decisions: Vec<PreparationDecision>,
}

pub(super) struct Costs<'a> {
    pub policy: InlineCostPolicy,
    model: &'a dyn CostModel,
    cache: HashMap<(String, usize), Option<GraphCost>>,
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
        let key = (symbol.to_owned(), 0);
        if let Some(cost) = self.cache.get(&key) {
            return *cost;
        }
        let cost = provider.preparation_graph(symbol).and_then(|source| {
            if source.live_count() > self.remaining_nodes {
                return None;
            }
            self.remaining_nodes -= source.live_count();
            self.nodes_visited += source.live_count();
            Some(estimate_graph(source, self.model))
        });
        self.cache.insert(key, cost);
        cost
    }

    pub fn estimate_source(
        &mut self,
        symbol: &str,
        specialization: usize,
        source: &FunctionGraph,
    ) -> Option<GraphCost> {
        if self.policy == InlineCostPolicy::Legacy {
            return None;
        }
        let key = (symbol.to_owned(), specialization);
        if let Some(cost) = self.cache.get(&key) {
            return *cost;
        }
        let cost = if source.live_count() <= self.remaining_nodes {
            self.remaining_nodes -= source.live_count();
            self.nodes_visited += source.live_count();
            Some(estimate_graph(source, self.model))
        } else {
            None
        };
        self.cache.insert(key, cost);
        cost
    }

    pub fn priority(
        &mut self,
        graph: &FunctionGraph,
        call: NodeId,
        provider: &(impl GraphProvider + ?Sized),
    ) -> u64 {
        let symbol = &graph.callees[graph.payload(call) as usize].symbol;
        if self.policy.uses_size() {
            self.estimate(symbol, provider)
                .and_then(|cost| cost.code_size.total())
                .unwrap_or(u64::MAX)
        } else {
            provider
                .preparation_graph(symbol)
                .map_or(0, FunctionGraph::live_count) as u64
        }
    }

    pub fn call_units(&self, graph: &FunctionGraph, call: NodeId) -> Option<u64> {
        use crate::analysis::cost::{CostKind, InstructionCost};
        if !matches!(
            self.policy,
            InlineCostPolicy::Residual { .. } | InlineCostPolicy::CallSiteResidual { .. }
        ) {
            return None;
        }
        match self.model.instruction_cost(graph, call, CostKind::CodeSize) {
            InstructionCost::Known(units) => Some(units),
            InstructionCost::Unknown => None,
        }
    }

    pub fn model_name(&self) -> &'static str {
        self.model.name()
    }
}
