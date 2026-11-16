//! Bounded wrapper preparation for the legacy preparation ablation. Published
//! bodies use the same crate-wide symbol and data ownership as ordinary CGSCC.
use super::{CompileError, CrateGraphs};
use kfrc_son::{FunctionGraph, GraphProvider};

impl CrateGraphs {
    /// Prepare small wrappers before they are priced at their callers. Each
    /// wave reads immutable prior-wave bodies; recursive inlining is rejected
    /// by the normal inliner. Keep shrinking results or small bodies with fewer calls; cap total work.
    pub(crate) fn simplify_wrappers(&mut self) -> Result<(), CompileError> {
        if std::env::var("KFRC_OPTIMIZE").as_deref() == Ok("0")
            || std::env::var("KFRC_INLINE").as_deref() == Ok("0")
        {
            return Ok(());
        }
        let mut fuel = 100_000usize;
        for _ in 0..2 {
            let provider = ScratchGraphs(self);
            let mut updates = Vec::new();
            for (symbol, source) in &self.bodies {
                let size = source.live_count();
                if size > 128 || !source.ids().any(|n| source.op(n) == kfrc_son::Opcode::Call) {
                    continue;
                }
                if fuel < size {
                    break;
                }
                fuel -= size;
                let fail = |error: String| CompileError {
                    symbol: symbol.clone(),
                    message: format!("wrapper preparation: {error}"),
                };
                let mut candidate = source.clone();
                let identities = kfrc_son::fold_identity_calls(&mut candidate, &provider, None)
                    .map_err(|e| fail(e.to_string()))?;
                let inlined = kfrc_son::inline_calls(
                    &mut candidate,
                    &provider,
                    kfrc_son::InlineBudget {
                        max_depth: 2,
                        max_nodes: 256,
                        max_calls: 4,
                        max_callee_nodes: 64,
                        copy_anonymous_constants: true,
                        ..kfrc_son::InlineBudget::default()
                    },
                )
                .map_err(|e| fail(e.to_string()))?;
                if identities.calls_folded + inlined.calls_inlined == 0 {
                    continue;
                }
                let seeds: Vec<_> = identities
                    .dirty_nodes
                    .into_iter()
                    .chain(inlined.dirty_nodes)
                    .collect();
                let slots: Vec<_> = identities
                    .argument_slots
                    .into_iter()
                    .chain(inlined.argument_slots)
                    .collect();
                kfrc_son::optimize_specialized_seams(&mut candidate, &seeds, &slots)
                    .map_err(|e| fail(e.to_string()))?;
                let calls = |g: &kfrc_son::FunctionGraph| {
                    g.ids()
                        .filter(|&n| g.op(n) == kfrc_son::Opcode::Call)
                        .count()
                };
                if candidate.live_count() < size
                    || (candidate.live_count() <= 64 && calls(&candidate) < calls(source))
                {
                    updates.push((symbol.clone(), candidate));
                }
            }
            if updates.is_empty() {
                break;
            }
            self.bodies.extend(updates);
            self.collect_references();
        }
        Ok(())
    }
}

pub(super) struct ScratchGraphs<'a>(pub(super) &'a CrateGraphs);

impl GraphProvider for ScratchGraphs<'_> {
    fn graph(&self, symbol: &str) -> Option<&FunctionGraph> {
        if self.0.no_inline.contains(symbol) {
            None
        } else {
            self.0.bodies.get(symbol)
        }
    }

    fn always_inline(&self, symbol: &str) -> bool {
        self.0.always_inline.contains(symbol)
    }

    fn sole_call_site(&self, _symbol: &str) -> bool {
        // Scratch has no owning output CGU and grants no size waiver.
        false
    }
}

#[cfg(test)]
mod tests;
