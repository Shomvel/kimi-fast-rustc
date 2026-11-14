//! DSE's operand proof shares the total-evaluation contract with promotion.
use super::DseStats;
use crate::analysis::value_evaluation::{EvaluationStats, TotalEvaluation};
use crate::{FunctionGraph, NodeId};

pub(super) struct StoreSafety {
    values: TotalEvaluation,
}

impl StoreSafety {
    pub(super) fn new(graph: &FunctionGraph) -> Self {
        Self {
            values: TotalEvaluation::new(graph),
        }
    }

    pub(super) fn operands_and_encoding(
        &mut self,
        graph: &FunctionGraph,
        store: NodeId,
        fuel: &mut usize,
        stats: &mut DseStats,
    ) -> bool {
        let mut evaluation = EvaluationStats::default();
        let safe = self
            .values
            .operands_and_encoding(graph, store, fuel, &mut evaluation);
        stats.node_visits += evaluation.node_visits;
        stats.walk_steps += evaluation.node_visits;
        stats.budget_bailouts += evaluation.budget_bailouts;
        safe
    }
}
