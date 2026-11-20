//! Budgeted same-crate inlining of verified specialized value graphs.
//!
//! The producer must run the original collector's required-constant and
//! mentioned-item diagnostics before calling this pass and independently prove
//! emission reachability. This pass does not replace that proof or definition-
//! level R specialization. Missing or ineligible callees stay calls.
mod argument_promotion;
mod arguments;
mod conditional_returns;
mod cost;
mod driver;
mod identity;
mod loop_constants;
mod plan;
mod scalar_clone;
mod splice;
mod tail_recursion;
mod types;

pub use argument_promotion::{ArgumentPromotionError, ArgumentPromotionStats, promote_arguments};
pub use scalar_clone::specialize_literal_argument;
pub use tail_recursion::{TailRecursionStats, eliminate_tail_recursion};

use crate::analysis::cost::{CostModel, GenericCostModel};
use cost::Costs;
pub use cost::{
    InlineCostPolicy, InlineDecision, InlineTemperature, PreparationDecision, PreparationStats,
};

pub use conditional_returns::{ConditionalReturnStats, ConditionalReturns};
pub use identity::{IdentityFoldStats, fold_identity_calls};

use crate::{FunctionGraph, NodeId, VerifyError};
use kfrc_body::fx_hash::HashMap;
use std::collections::BTreeMap;

pub trait GraphProvider {
    fn graph(&self, symbol: &str) -> Option<&FunctionGraph>;
    /// Prefer this callee and waive ordinary size/cost profitability limits.
    /// Hard budgets, source availability and every splice legality check remain.
    fn always_inline(&self, _symbol: &str) -> bool {
        false
    }
    /// Source visible to bounded call-site preparation. A body may initially
    /// reference private symbols that constant control pruning can discard.
    /// `can_reference` still guards every reference in the final splice.
    fn preparation_graph(&self, symbol: &str) -> Option<&FunctionGraph> {
        self.graph(symbol)
    }
    /// Whether the destination object can link a surviving reference.
    fn can_reference(&self, _symbol: &str) -> bool {
        true
    }
    /// The callee is local to the output and this is its only reference, so
    /// inlining it removes the out-of-line body (LLVM's last-call-to-static).
    fn sole_call_site(&self, _symbol: &str) -> bool {
        false
    }
}
impl GraphProvider for BTreeMap<String, FunctionGraph> {
    fn graph(&self, symbol: &str) -> Option<&FunctionGraph> {
        self.get(symbol)
    }
}
impl GraphProvider for HashMap<String, FunctionGraph> {
    fn graph(&self, symbol: &str) -> Option<&FunctionGraph> {
        self.get(symbol)
    }
}

/// Strong inline intent precedes ordinary smallest-callee-first ordering.
/// Callees of at most `free_nodes` replace little more than the call sequence
/// and do not count against `max_calls`; callees above `max_callee_nodes`
/// stay calls unless this is their sole call site or they have strong intent.
#[derive(Clone, Copy, Debug)]
pub struct InlineBudget {
    pub max_depth: usize,
    pub max_nodes: usize,
    pub max_calls: usize,
    pub free_nodes: usize,
    pub max_callee_nodes: usize,
    /// Copy read-only anonymous constants (`unnamed_address`) at every site.
    /// Off by default: the node-count cost model has no cold-path signal, and
    /// semver measured +11 KB text and a slower hot loop with it on.
    pub copy_anonymous_constants: bool,
    /// Alignment-inclusive constant bytes imported at cyclic call sites and
    /// their nested calls. Zero disables this bounded profitability policy.
    /// Object identity, relocation visibility and ordinary node limits still
    /// apply; the explicit copy option above retains its existing behavior.
    pub max_loop_constant_bytes: usize,
}
impl Default for InlineBudget {
    fn default() -> Self {
        Self {
            max_depth: 4,
            max_nodes: 2000,
            max_calls: 32,
            free_nodes: 24,
            max_callee_nodes: 64,
            copy_anonymous_constants: false,
            max_loop_constant_bytes: 1024,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InlineSkip {
    Missing,
    Recursive,
    Budget,
    /// Larger than `max_callee_nodes` and called from elsewhere too.
    Size,
    /// Known code-size estimate exceeds the selected policy threshold.
    Cost,
    /// The model cannot price this callee within its analysis budget.
    UnknownCost,
    Unsupported,
    TypeConflict,
    AbiMismatch,
    DataConflict,
    /// A reference surviving call-site pruning is private to another object.
    PrivateReference,
}
#[derive(Clone, Debug, Default)]
pub struct InlineStats {
    pub calls_inlined: usize,
    pub decisions: Vec<InlineDecision>,
    /// Cost summaries are cached per callee and bounded per caller.
    pub cost_nodes_visited: usize,
    /// The persistent scalar seam index, including its initial build and
    /// changed/new nodes. It is rebuilt only after arbitrary CFG edits.
    pub seam_nodes_indexed: usize,
    pub seam_node_visits: usize,
    pub seam_control_cleanups: usize,
    pub seam_allocation_bailouts: usize,
    pub nodes_added: usize,
    pub skipped: Vec<(String, InlineSkip)>,
    /// Nodes inspected by eligibility, recursion discovery and splice planning.
    pub nodes_visited: usize,
    /// New local constant storage, including worst-case alignment padding.
    pub constant_bytes_imported: usize,
    /// Bounded original-CFG indexing and cycle-membership work.
    pub loop_constant_work: usize,
    /// Reserved weighted units for bounded S call-site classification.
    pub temperature_work: usize,
    pub temperature_budget_bailouts: usize,
    /// Estimated growth above the ordinary threshold spent by hot splices.
    /// This account is independent of actual allocated graph nodes/work.
    pub hot_growth_spent: u64,
    /// Source graph nodes copied, excluding call-boundary and lifetime nodes.
    pub cloned_nodes: usize,
    /// Stable until the caller compacts: seam users, returns and lifetime nodes.
    pub dirty_nodes: Vec<NodeId>,
    /// Caller LocalSlot roots passed to an erased call (W06/F08).
    pub argument_slots: Vec<NodeId>,
    /// Call-site candidates are bounded separately from emitted graph growth.
    pub specializations_prepared: usize,
    pub specialization_nodes_cloned: usize,
    pub specialization_cache_hits: usize,
    /// Work performed in disposable call-site candidates, separate from emission.
    pub preparation: PreparationStats,
}
impl InlineStats {
    fn reject(&mut self, symbol: String, reason: InlineSkip, decision: Option<usize>) {
        if let Some(index) = decision {
            self.decisions[index].rejection = Some(reason);
        }
        self.skipped.push((symbol, reason));
    }
}

#[derive(Debug)]
pub enum InlineError {
    NotSpecialized,
    Verification(VerifyError),
    Specialization(crate::PipelineError),
}
impl std::fmt::Display for InlineError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "semantic inlining: {self:?}")
    }
}
impl std::error::Error for InlineError {}

pub fn inline_calls(
    caller: &mut FunctionGraph,
    provider: &(impl GraphProvider + ?Sized),
    budget: InlineBudget,
) -> Result<InlineStats, InlineError> {
    inline_calls_with_cost(
        caller,
        provider,
        budget,
        InlineCostPolicy::Legacy,
        &GenericCostModel,
    )
}

/// Run with an explicit cost policy. Observe is decision-identical to Legacy;
/// Explicit policies leave the default library helper on Legacy pricing.
pub fn inline_calls_with_cost(
    caller: &mut FunctionGraph,
    provider: &(impl GraphProvider + ?Sized),
    budget: InlineBudget,
    policy: InlineCostPolicy,
    model: &dyn CostModel,
) -> Result<InlineStats, InlineError> {
    let mut arguments = arguments::Arguments::new(budget.max_nodes);
    let mut costs = Costs::new(policy, model, budget.max_nodes);
    let mut stats = driver::run(
        caller,
        provider,
        budget,
        &mut arguments,
        &mut costs,
        driver::Frame::Emission,
    )?;
    stats.cost_nodes_visited = costs.nodes_visited;
    arguments.finish(&mut stats);
    Ok(stats)
}
