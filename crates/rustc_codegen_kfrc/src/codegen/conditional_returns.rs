//! Expose a proved, effect-free return path without cloning the other arm.

use super::{CompileError, CrateGraphs};

impl CrateGraphs {
    pub(crate) fn infer_conditional_returns(&mut self) {
        self.conditional_returns = kfrc_son::ConditionalReturns::default();
        if std::env::var("KFRC_OPTIMIZE").as_deref() == Ok("0")
            || std::env::var("KFRC_CONDITIONAL_RETURNS").as_deref() == Ok("0")
        {
            return;
        }
        // Promotion has completed and local inlining preserves each callee's
        // semantics and signature. The table belongs to this crate snapshot;
        // neither FunctionGraph nor CallTarget stores the proof.
        self.conditional_returns =
            kfrc_son::ConditionalReturns::infer_exact(&self.bodies, &self.internal);
    }

    pub(crate) fn simplify_conditional_returns(
        &self,
        graph: &mut kfrc_son::FunctionGraph,
    ) -> Result<(), CompileError> {
        if !self.conditional_returns.has_candidates(graph) {
            return Ok(());
        }
        let stats = self
            .conditional_returns
            .fold_calls(graph)
            .map_err(|error| CompileError {
                symbol: graph.symbol.clone(),
                message: format!("conditional returns: {error}"),
            })?;
        if stats.calls_split != 0 {
            kfrc_son::optimize_specialized_seams(graph, &stats.dirty_nodes, &stats.argument_slots)
                .map_err(|error| CompileError {
                    symbol: graph.symbol.clone(),
                    message: format!("conditional return cleanup: {error}"),
                })?;
        }
        if std::env::var_os("KFRC_SPECIALIZATION_STATS").is_some()
            || std::env::var_os("KFRC_DUMP_GRAPH").is_some()
        {
            eprintln!("kfrc conditional returns {}: {stats:?}", graph.symbol);
        }
        Ok(())
    }
}
