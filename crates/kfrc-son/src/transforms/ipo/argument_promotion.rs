//! Promote read-only pointer parameters of private, directly called functions.
//!
//! The producer supplies the closed set of private definitions. This pass also
//! rejects every address reference visible in those graphs and their data.
mod plan;
mod rewrite;

use std::collections::{BTreeMap, BTreeSet};

use crate::{FunctionGraph, MemoryPartition, Opcode, VerifyError};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ArgumentPromotionStats {
    pub functions_changed: usize,
    pub arguments_promoted: usize,
    pub scalar_parts: usize,
    pub calls_changed: usize,
}

#[derive(Debug)]
pub struct ArgumentPromotionError {
    pub symbol: String,
    pub error: VerifyError,
}

impl std::fmt::Display for ArgumentPromotionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "argument promotion of {}: {}", self.symbol, self.error)
    }
}
impl std::error::Error for ArgumentPromotionError {}

/// `private` must exclude definitions referenced outside `bodies`, including
/// static/vtable relocations and machine-only adapters. All its callers must
/// be represented here. Exported and address-taken functions keep their ABI.
pub fn promote_arguments(
    bodies: &mut BTreeMap<String, FunctionGraph>,
    private: &BTreeSet<String>,
) -> Result<ArgumentPromotionStats, ArgumentPromotionError> {
    let mut stats = ArgumentPromotionStats::default();
    let mut callers: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut addressed = BTreeSet::new();
    for (owner, graph) in bodies.iter() {
        for node in graph.ids() {
            match graph.op(node) {
                Opcode::Call => {
                    callers
                        .entry(graph.callees[graph.payload(node) as usize].symbol.clone())
                        .or_default()
                        .insert(owner.clone());
                }
                Opcode::SymbolAddress | Opcode::ThreadLocalAddress => {
                    addressed.insert(graph.symbols[graph.payload(node) as usize].0.clone());
                }
                _ => {}
            }
        }
        for partition in &graph.memory_partitions {
            if let MemoryPartition::Const { symbol } = *partition {
                addressed.insert(graph.symbols[symbol as usize].0.clone());
            }
        }
        for object in graph.data_definitions() {
            addressed.extend(object.relocations.iter().map(|r| r.target.clone()));
        }
    }
    // Node identities stay stable across the batch. No inlining or compaction
    // occurs here, and promotion neither creates nor removes a call edge.
    for symbol in private.difference(&addressed) {
        let Some(source) = bodies.get(symbol) else {
            continue;
        };
        let Some(owners) = callers.get(symbol) else {
            continue;
        };
        let Some(plan) = plan::build(source) else {
            continue;
        };
        let mut updates = BTreeMap::new();
        let mut changed_calls = 0;
        let mut compatible = true;
        for owner in owners {
            let mut caller = bodies[owner].clone();
            if let Some(count) = rewrite::calls(&mut caller, source, &plan) {
                changed_calls += count;
                updates.insert(owner.clone(), caller);
            } else {
                compatible = false;
                break;
            }
        }
        if !compatible {
            continue;
        }
        let mut callee = updates.remove(symbol).unwrap_or_else(|| source.clone());
        rewrite::definition(&mut callee, &plan);
        updates.insert(symbol.clone(), callee);
        // The signature edit is transactional over the definition and every
        // caller; an import or verification failure cannot publish half an ABI.
        for (owner, graph) in &mut updates {
            graph.resolve_edges();
            crate::verify::verify_pipeline_step(graph).map_err(|error| ArgumentPromotionError {
                symbol: owner.clone(),
                error,
            })?;
        }
        stats.functions_changed += 1;
        stats.arguments_promoted += plan.arguments.len();
        stats.scalar_parts += plan
            .arguments
            .iter()
            .map(|arg| arg.parts.len())
            .sum::<usize>();
        stats.calls_changed += changed_calls;
        bodies.extend(updates);
    }
    Ok(stats)
}
