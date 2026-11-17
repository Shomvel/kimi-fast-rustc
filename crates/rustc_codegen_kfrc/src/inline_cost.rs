//! Residual pricing for shared prepared bodies and optional machine-readable
//! decisions. Profitability remains separate from compiler-work limits.
use std::sync::OnceLock;

use kfrc_son::{InlineBudget, InlineCostPolicy, InlineStats};

use crate::profile::json_string;

/// Graph nodes one inliner invocation may add, unless configured.
pub(crate) const DEFAULT_INVOCATION_NODES: usize = 65_536;
/// Compiler-work units of the bottom-up preparation, unless configured.
pub(crate) const DEFAULT_PREPARATION_WORK: usize = 1_000_000;
/// Positive residual growth accepted at ordinary production call sites.
const DEFAULT_ORDINARY_GROWTH: u64 = 160;
/// Preserve the explicit uniform-residual calibration contract.
const UNIFORM_RESIDUAL_GROWTH: u64 = 32;
const HOT_RESIDUAL_GROWTH: u64 = 256;
/// Extra estimated growth shared by hot splices in one emission invocation.
const TOTAL_HOT_RESIDUAL_GROWTH: u64 = 512;

/// The inline limits of this process: the accepted defaults, each replaceable
/// through the environment for calibration runs.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Limits {
    /// Base budget of every inliner invocation.
    pub budget: InlineBudget,
    /// Graph nodes one inliner invocation may add.
    pub invocation_nodes: usize,
    /// Compiler-work units of the crate's bottom-up preparation.
    pub preparation_work: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            budget: InlineBudget {
                max_depth: 1,
                ..InlineBudget::default()
            },
            invocation_nodes: DEFAULT_INVOCATION_NODES,
            preparation_work: DEFAULT_PREPARATION_WORK,
        }
    }
}

pub(crate) fn limits() -> Result<Limits, String> {
    static LIMITS: OnceLock<Result<Limits, String>> = OnceLock::new();
    LIMITS.get_or_init(read_limits).clone()
}

fn read_limits() -> Result<Limits, String> {
    let number = |name: &str, default: usize| match std::env::var(name) {
        Err(std::env::VarError::NotPresent) => Ok(default),
        Ok(text) => text
            .parse::<usize>()
            .map_err(|_| format!("invalid {name}={text:?}; expected a non-negative integer")),
        Err(error) => Err(format!("invalid {name}: {error}")),
    };
    let defaults = Limits::default();
    let budget = defaults.budget;
    Ok(Limits {
        budget: InlineBudget {
            max_depth: number("KFRC_INLINE_DEPTH", budget.max_depth)?,
            max_nodes: number("KFRC_INLINE_NODES", budget.max_nodes)?,
            max_calls: number("KFRC_INLINE_CALLS", budget.max_calls)?,
            free_nodes: number("KFRC_INLINE_FREE_NODES", budget.free_nodes)?,
            max_callee_nodes: number("KFRC_INLINE_CALLEE_NODES", budget.max_callee_nodes)?,
            copy_anonymous_constants: number(
                "KFRC_INLINE_COPY_CONSTANTS",
                usize::from(budget.copy_anonymous_constants),
            )? != 0,
            max_loop_constant_bytes: number(
                "KFRC_INLINE_LOOP_CONSTANT_BYTES",
                budget.max_loop_constant_bytes,
            )?,
        },
        invocation_nodes: number("KFRC_INLINE_INVOCATION_NODES", defaults.invocation_nodes)?,
        preparation_work: number("KFRC_INLINE_PREPARATION_WORK", defaults.preparation_work)?,
    })
}

pub(crate) fn policy() -> Result<InlineCostPolicy, String> {
    match std::env::var("KFRC_INLINE_COST_MODEL") {
        Err(std::env::VarError::NotPresent) => residual_policy(true),
        Ok(value) => match value.as_str() {
            "legacy" => Ok(InlineCostPolicy::Legacy),
            "observe" => Ok(InlineCostPolicy::Observe),
            // Relative units, not bytes. These are explicit experimental
            // thresholds; correctness/growth/recursion constraints still apply.
            "size" => Ok(InlineCostPolicy::CodeSize {
                max_units: 64,
                free_units: 16,
            }),
            "residual" => residual_policy(false),
            "callsite" => residual_policy(true),
            _ => Err(format!(
                "invalid KFRC_INLINE_COST_MODEL={value:?}; expected legacy, observe, size, residual or callsite"
            )),
        },
        Err(error) => Err(format!("invalid KFRC_INLINE_COST_MODEL: {error}")),
    }
}

/// Experimental call-site thresholds; unset variables keep the accepted defaults.
fn growth(name: &str, default: u64) -> Result<u64, String> {
    match std::env::var(name) {
        Err(std::env::VarError::NotPresent) => Ok(default),
        Ok(text) => text
            .parse::<u64>()
            .map_err(|_| format!("invalid {name}={text:?}; expected a non-negative integer")),
        Err(error) => Err(format!("invalid {name}: {error}")),
    }
}

fn residual_policy(callsite: bool) -> Result<InlineCostPolicy, String> {
    // An explicit uniform threshold retains the original what-if contract.
    // Environment changes are read at process setup, never during analysis.
    match std::env::var("KFRC_INLINE_RESIDUAL_UNITS") {
        Ok(value) => value
            .parse::<u64>()
            .map(|max_growth| InlineCostPolicy::Residual { max_growth })
            .map_err(|_| {
                format!(
                    "invalid KFRC_INLINE_RESIDUAL_UNITS={value:?}; expected a non-negative integer"
                )
            }),
        Err(std::env::VarError::NotPresent) if callsite => Ok(InlineCostPolicy::CallSiteResidual {
            max_growth: growth("KFRC_INLINE_ORDINARY_GROWTH", DEFAULT_ORDINARY_GROWTH)?,
            hot_growth: growth("KFRC_INLINE_HOT_GROWTH", HOT_RESIDUAL_GROWTH)?,
            total_hot_growth: growth("KFRC_INLINE_TOTAL_HOT_GROWTH", TOTAL_HOT_RESIDUAL_GROWTH)?,
        }),
        Err(std::env::VarError::NotPresent) => Ok(InlineCostPolicy::Residual {
            max_growth: UNIFORM_RESIDUAL_GROWTH,
        }),
        Err(error) => Err(format!("invalid KFRC_INLINE_RESIDUAL_UNITS: {error}")),
    }
}

pub(crate) fn model(policy: InlineCostPolicy) -> &'static dyn kfrc_son::analysis::cost::CostModel {
    match policy {
        InlineCostPolicy::Residual { .. } | InlineCostPolicy::CallSiteResidual { .. } => {
            &kfrc_son::analysis::cost::ResidualCostModel
        }
        _ => &kfrc_son::analysis::cost::GenericCostModel,
    }
}

pub(crate) fn report(caller: &str, stats: &InlineStats, policy: InlineCostPolicy) {
    if policy != InlineCostPolicy::Observe
        && std::env::var_os("KFRC_CGSCC_STATS").is_none()
        && std::env::var_os("KFRC_SPECIALIZATION_STATS").is_none()
        && std::env::var_os("KFRC_DUMP_GRAPH").is_none()
    {
        return;
    }
    if stats.seam_nodes_indexed != 0 {
        eprintln!(
            "kfrc-inline-seams {{\"schema\":1,\"caller\":{},\"nodes_indexed\":{},\"node_visits\":{},\"control_cleanups\":{},\"allocation_bailouts\":{}}}",
            json_string(caller),
            stats.seam_nodes_indexed,
            stats.seam_node_visits,
            stats.seam_control_cleanups,
            stats.seam_allocation_bailouts,
        );
    }
    if stats.temperature_work != 0 || stats.temperature_budget_bailouts != 0 {
        eprintln!(
            "kfrc-inline-temperature {{\"schema\":1,\"caller\":{},\"work\":{},\"budget_bailouts\":{},\"hot_growth_spent\":{}}}",
            json_string(caller),
            stats.temperature_work,
            stats.temperature_budget_bailouts,
            stats.hot_growth_spent,
        );
    }
    if stats.constant_bytes_imported != 0 || stats.loop_constant_work != 0 {
        eprintln!(
            "kfrc-inline-constants {{\"schema\":1,\"caller\":{},\"constant_bytes_imported\":{},\"loop_constant_work\":{}}}",
            json_string(caller),
            stats.constant_bytes_imported,
            stats.loop_constant_work,
        );
    }
    let preparation = &stats.preparation;
    let preparation_summary = format!(
        "{{\"calls_inlined\":{},\"costed_calls_inlined\":{},\"free_calls_inlined\":{},\"cache_hits\":{},\"cached_calls_inlined\":{},\"cached_splice_nodes_allocated\":{},\"nodes_visited\":{},\"priority_visit_budget\":{},\"priority_node_visits\":{},\"priority_budget_bailouts\":{},\"fact_reapplication_visit_budget\":{},\"fact_reapplication_node_visits\":{},\"fact_reapplication_fields_replaced\":{},\"fact_reapplication_nodes_allocated\":{},\"fact_reapplication_budget_bailouts\":{},\"fact_node_visits\":{},\"clone_budget_used\":{},\"source_nodes_cloned\":{},\"transaction_nodes_cloned\":{},\"splice_nodes_cloned\":{},\"splice_nodes_allocated\":{},\"cleanup_runs\":{},\"cleanup_rounds\":{},\"cleanup_nodes_indexed\":{},\"cleanup_node_visits\":{},\"cleanup_nodes_added\":{},\"cleanup_branches_folded\":{},\"cleanup_observations_removed\":{},\"cleanup_conditions_folded\":{},\"maximum_depth\":{},\"budget_bailouts\":{},\"decisions_dropped\":{}}}",
        preparation.calls_inlined,
        preparation.costed_calls_inlined,
        preparation.free_calls_inlined,
        preparation.cache_hits,
        preparation.cached_calls_inlined,
        preparation.cached_splice_nodes_allocated,
        preparation.nodes_visited,
        preparation.priority_visit_budget,
        preparation.priority_node_visits,
        preparation.priority_budget_bailouts,
        preparation.fact_reapplication_visit_budget,
        preparation.fact_reapplication_node_visits,
        preparation.fact_reapplication_fields_replaced,
        preparation.fact_reapplication_nodes_allocated,
        preparation.fact_reapplication_budget_bailouts,
        preparation.fact_node_visits,
        preparation.clone_budget_used,
        preparation.source_nodes_cloned,
        preparation.transaction_nodes_cloned,
        preparation.splice_nodes_cloned,
        preparation.splice_nodes_allocated,
        preparation.cleanup_runs,
        preparation.cleanup_rounds,
        preparation.cleanup_nodes_indexed,
        preparation.cleanup_node_visits,
        preparation.cleanup_nodes_added,
        preparation.cleanup_branches_folded,
        preparation.cleanup_observations_removed,
        preparation.cleanup_conditions_folded,
        preparation.maximum_depth,
        preparation.budget_bailouts,
        preparation.decisions_dropped,
    );
    if preparation.cleanup_runs != 0 || preparation.budget_bailouts != 0 {
        eprintln!(
            "kfrc-preparation-summary {{\"schema\":1,\"caller\":{},\"work\":{}}}",
            json_string(caller),
            preparation_summary
        );
    }
    for prepared in &preparation.decisions {
        let specialized = prepared
            .specialized_nodes
            .map_or_else(|| "null".into(), |nodes| nodes.to_string());
        let rejection = prepared.rejection.map_or_else(
            || "null".into(),
            |reason| json_string(&format!("{reason:?}")),
        );
        eprintln!(
            "kfrc-preparation {{\"schema\":1,\"caller\":{},\"root\":{},\"owner\":{},\"callee\":{},\"depth\":{},\"source_nodes\":{},\"specialized_nodes\":{},\"action\":{},\"rejection\":{}}}",
            json_string(caller),
            json_string(&prepared.root),
            json_string(&prepared.owner),
            json_string(&prepared.symbol),
            prepared.depth,
            prepared.source_nodes,
            specialized,
            json_string(prepared.action),
            rejection
        );
    }
    for decision in &stats.decisions {
        let estimate = decision.estimate.map_or_else(|| "null".into(), |cost| format!(
            "{{\"nodes\":{},\"input_edges\":{},\"size_units\":{},\"size_unknown\":{},\"static_work_units\":{},\"work_unknown\":{}}}",
            cost.nodes, cost.input_edges, cost.code_size.known_units, cost.code_size.unknown_nodes,
            cost.static_work.known_units, cost.static_work.unknown_nodes,
        ));
        let budget = format!(
            "{{\"max_depth\":{},\"max_nodes\":{},\"max_calls\":{},\"max_callee_nodes\":{},\"free_nodes\":{},\"nodes_added_before\":{},\"costed_calls_before\":{}}}",
            decision.budget.max_depth,
            decision.budget.max_nodes,
            decision.budget.max_calls,
            decision.budget.max_callee_nodes,
            decision.budget.free_nodes,
            decision.nodes_added_before,
            decision.costed_calls_before,
        );
        let outcome = decision
            .rejection
            .map_or_else(|| "inlined".into(), |reason| format!("{reason:?}"));
        let specialized_nodes = decision
            .specialized_nodes
            .map_or_else(|| "null".into(), |nodes| nodes.to_string());
        let hot_remaining = decision
            .hot_growth_remaining
            .map_or_else(|| "null".into(), |units| units.to_string());
        let (policy, maximum, free_limit) = match decision.policy {
            InlineCostPolicy::Legacy => ("legacy", "null".into(), "null".into()),
            InlineCostPolicy::Observe => ("observe", "null".into(), "null".into()),
            InlineCostPolicy::CodeSize {
                max_units,
                free_units,
            } => ("size", max_units.to_string(), free_units.to_string()),
            InlineCostPolicy::Residual { max_growth }
            | InlineCostPolicy::CallSiteResidual { max_growth, .. } => (
                "residual",
                decision
                    .call_units
                    .and_then(|call| call.checked_add(max_growth))
                    .map_or_else(|| "null".into(), |units| units.to_string()),
                decision
                    .call_units
                    .map_or_else(|| "null".into(), |units| units.to_string()),
            ),
        };
        eprintln!(
            "kfrc-cost {{\"schema\":1,\"caller\":{},\"callee\":{},\"call\":{},\"depth\":{},\"source_nodes\":{},\"specialized_nodes\":{},\"model\":{},\"policy\":{},\"temperature\":{},\"hot_growth_remaining\":{},\"hot_growth_charged\":{},\"max_size_units\":{},\"free_size_units\":{},\"sole_call_site\":{},\"free\":{},\"estimate\":{},\"budget\":{},\"outcome\":{},\"analysis_nodes_for_caller\":{},\"specializations_for_caller\":{},\"specialization_nodes_cloned\":{},\"specialization_cache_hits\":{}}}",
            json_string(caller),
            json_string(&decision.symbol),
            decision.call.0,
            decision.depth,
            decision.source_nodes,
            specialized_nodes,
            json_string(decision.model),
            json_string(policy),
            json_string(decision.temperature.as_str()),
            hot_remaining,
            decision.hot_growth_charged,
            maximum,
            free_limit,
            decision.sole_call_site,
            decision.free,
            estimate,
            budget,
            json_string(&outcome),
            stats.cost_nodes_visited,
            stats.specializations_prepared,
            stats.specialization_nodes_cloned,
            stats.specialization_cache_hits,
        );
    }
}
