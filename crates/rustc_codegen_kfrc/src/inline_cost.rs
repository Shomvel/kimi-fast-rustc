//! Opt-in cost policy and machine-readable decision explanations. The default
//! remains the accepted node policy until the seed model is calibrated.
use kfrc_son::{InlineCostPolicy, InlineStats};

use crate::profile::json_string;

pub(crate) fn policy() -> Result<InlineCostPolicy, String> {
    match std::env::var("KFRC_INLINE_COST_MODEL") {
        Err(std::env::VarError::NotPresent) => Ok(InlineCostPolicy::Legacy),
        Ok(value) => match value.as_str() {
            "legacy" => Ok(InlineCostPolicy::Legacy),
            "observe" => Ok(InlineCostPolicy::Observe),
            // Relative units, not bytes. These are explicit experimental
            // thresholds; correctness/growth/recursion constraints still apply.
            "size" => Ok(InlineCostPolicy::CodeSize {
                max_units: 64,
                free_units: 16,
            }),
            _ => Err(format!(
                "invalid KFRC_INLINE_COST_MODEL={value:?}; expected legacy, observe or size"
            )),
        },
        Err(error) => Err(format!("invalid KFRC_INLINE_COST_MODEL: {error}")),
    }
}

pub(crate) fn report(caller: &str, stats: &InlineStats) {
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
        let (policy, maximum, free_limit) = match decision.policy {
            InlineCostPolicy::Legacy => ("legacy", "null".into(), "null".into()),
            InlineCostPolicy::Observe => ("observe", "null".into(), "null".into()),
            InlineCostPolicy::CodeSize {
                max_units,
                free_units,
            } => ("size", max_units.to_string(), free_units.to_string()),
        };
        eprintln!(
            "kfrc-cost {{\"schema\":1,\"caller\":{},\"callee\":{},\"call\":{},\"depth\":{},\"source_nodes\":{},\"model\":{},\"policy\":{},\"max_size_units\":{},\"free_size_units\":{},\"sole_call_site\":{},\"free\":{},\"estimate\":{},\"budget\":{},\"outcome\":{},\"analysis_nodes_for_caller\":{}}}",
            json_string(caller),
            json_string(&decision.symbol),
            decision.call.0,
            decision.depth,
            decision.source_nodes,
            json_string(decision.model),
            json_string(policy),
            maximum,
            free_limit,
            decision.sole_call_site,
            decision.free,
            estimate,
            budget,
            json_string(&outcome),
            stats.cost_nodes_visited,
        );
    }
}
