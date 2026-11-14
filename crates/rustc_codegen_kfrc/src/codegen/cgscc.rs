//! Prepare one crate-wide callee body before pricing its callers. Symbol
//! ownership and visibility are assigned after optimization, during emission.
mod report;
mod schedule;

use super::{CompileError, CrateGraphs};
use crate::{emission, inline_cost};
use kfrc_son::{FunctionGraph, GraphProvider, InlineBudget, InlineSkip, InlineStats, Opcode};
use std::collections::{BTreeMap, BTreeSet};

// The preparation work limit (`inline_cost::Limits::preparation_work`) counts
// compiler-work units, not emitted nodes or CPU instructions. Admission stops
// when this account is spent; a bounded inliner invocation is charged in full
// before another operation starts. Ordinary inline profitability is unchanged.
//
#[cfg(test)]
const MAX_WORK: usize = inline_cost::DEFAULT_PREPARATION_WORK;
#[cfg(test)]
const MAX_INLINE_NODES: usize = inline_cost::DEFAULT_INVOCATION_NODES;

impl CrateGraphs {
    pub(crate) fn prepare_callees(
        &mut self,
        jobs: &[emission::CguJob],
        adapters: &BTreeMap<String, crate::machine::CompiledFunction>,
    ) -> Result<(), CompileError> {
        self.inlining_complete.clear();
        if std::env::var("KFRC_CGSCC").as_deref() == Ok("0") {
            // Keep the accepted preparation and emission path byte-for-byte
            // available for ablation; do not run both preparation strategies.
            return self.simplify_wrappers();
        }
        if std::env::var("KFRC_OPTIMIZE").as_deref() == Ok("0")
            || std::env::var("KFRC_INLINE").as_deref() == Ok("0")
        {
            return Ok(());
        }
        let mut roots = BTreeSet::new();
        for job in jobs {
            roots.extend(job.needed.iter().cloned());
            emission::data_references(&job.data_objects, &mut roots);
        }
        for (function, objects) in adapters.values() {
            emission::function_references(function, &mut roots);
            emission::data_references(objects, &mut roots);
        }
        let limits = inline_cost::limits().map_err(|message| CompileError {
            symbol: "crate".into(),
            message,
        })?;
        run(self, &roots, limits.preparation_work).map(|_| ())
    }
}

fn reachable(graphs: &CrateGraphs, roots: &BTreeSet<String>) -> BTreeSet<String> {
    let mut pending = roots.clone();
    graphs.data.expand_references(&mut pending);
    let mut seen = BTreeSet::new();
    while let Some(symbol) = pending.pop_first() {
        if !graphs.bodies.contains_key(&symbol) || !seen.insert(symbol.clone()) {
            continue;
        }
        if let Some(references) = graphs.references.get(&symbol) {
            pending.extend(
                references
                    .iter()
                    .filter(|name| !seen.contains(*name))
                    .cloned(),
            );
        }
    }
    seen
}

// Completed acyclic visits expose their final body. Incomplete visits retain
// the source so a caller can still simplify it using its argument facts.
// Unvisited and recursive sources remain on the ordinary emission path.
struct PreparedGraphs<'a> {
    graphs: &'a CrateGraphs,
    ready: &'a BTreeSet<String>,
}

impl GraphProvider for PreparedGraphs<'_> {
    fn graph(&self, symbol: &str) -> Option<&FunctionGraph> {
        (self.ready.contains(symbol) && !self.graphs.no_inline.contains(symbol))
            .then(|| self.graphs.bodies.get(symbol))
            .flatten()
    }

    fn always_inline(&self, symbol: &str) -> bool {
        self.graphs.always_inline.contains(symbol)
    }
}

struct Work {
    limit: usize,
    used: usize,
}

impl Work {
    fn remaining(&self) -> usize {
        self.limit.saturating_sub(self.used)
    }

    fn reserve(&mut self, nodes: usize) -> bool {
        if nodes > self.remaining() {
            return false;
        }
        self.account(nodes);
        true
    }

    fn account(&mut self, nodes: usize) {
        self.used = self.used.saturating_add(nodes);
    }
}

struct Attempt {
    graph: Option<FunctionGraph>,
    identity_calls: usize,
    inlined: InlineStats,
}

fn prepare(
    source: &FunctionGraph,
    provider: &PreparedGraphs<'_>,
    work: &mut Work,
    policy: kfrc_son::InlineCostPolicy,
    limits: &inline_cost::Limits,
) -> Result<Attempt, CompileError> {
    let fail = |stage: &str, error: String| CompileError {
        symbol: source.symbol.clone(),
        message: format!("bottom-up {stage}: {error}"),
    };
    let mut attempt = Attempt {
        graph: None,
        identity_calls: 0,
        inlined: InlineStats::default(),
    };
    if !work.reserve(source.len()) {
        return Ok(attempt);
    }
    let mut graph = source.clone();
    let identities = kfrc_son::fold_identity_calls(&mut graph, provider, None)
        .map_err(|error| fail("identity", error.to_string()))?;
    attempt.identity_calls += identities.calls_folded;
    work.account(identities.summary_nodes_visited);
    if identities.calls_folded != 0 {
        work.account(source.len());
    }
    if work.remaining() == 0 {
        return Ok(attempt);
    }
    let before_inline = graph.len();
    attempt.inlined = kfrc_son::inline_calls_with_cost(
        &mut graph,
        provider,
        InlineBudget {
            max_nodes: limits.invocation_nodes.min(work.remaining()),
            ..limits.budget
        },
        policy,
        inline_cost::model(policy),
    )
    .map_err(|error| fail("inlining", error.to_string()))?;
    work.account(attempt.inlined.nodes_visited);
    work.account(attempt.inlined.cost_nodes_visited);
    work.account(attempt.inlined.specialization_nodes_cloned);
    work.account(attempt.inlined.nodes_added);
    work.account(attempt.inlined.preparation.nodes_visited);
    work.account(attempt.inlined.preparation.cleanup_nodes_indexed);
    work.account(attempt.inlined.preparation.cleanup_node_visits);
    work.account(attempt.inlined.loop_constant_work);
    if attempt.inlined.calls_inlined != 0 {
        work.account(before_inline);
    }
    // Keep existing cost/preparation diagnostics, even for discarded scratch.
    inline_cost::report(&source.symbol, &attempt.inlined, policy);
    let new_calls: Vec<_> = (before_inline..graph.len())
        .map(|index| kfrc_son::NodeId(index as u32))
        .filter(|node| !graph.is_dead(*node) && graph.op(*node) == Opcode::Call)
        .collect();
    if !new_calls.is_empty() && work.remaining() != 0 {
        let identities = kfrc_son::fold_identity_calls(&mut graph, provider, Some(&new_calls))
            .map_err(|error| fail("nested identity", error.to_string()))?;
        attempt.identity_calls += identities.calls_folded;
        work.account(identities.summary_nodes_visited);
        if identities.calls_folded != 0 {
            work.account(graph.len());
        }
    }
    if attempt.identity_calls + attempt.inlined.calls_inlined != 0 {
        // Initial S preparation already simplified the unchanged interior.
        // Repeat the complete pipeline only after a real call substitution.
        if !work.reserve(graph.len()) {
            return Ok(attempt);
        }
        kfrc_son::optimize_specialized_graph(&mut graph)
            .map_err(|error| fail("simplification", error.to_string()))?;
    }
    attempt.graph = Some(graph);
    Ok(attempt)
}

fn emission_needs_retry(
    graph: &FunctionGraph,
    provider: &PreparedGraphs<'_>,
    inlined: &InlineStats,
) -> bool {
    // Final emission can use crate-wide sole-call counts and bodies that were
    // not ready during this visit. Retry only residual calls so successful
    // substitutions are shared with every caller.
    inlined.skipped.iter().any(|(_, reason)| {
        matches!(
            reason,
            InlineSkip::Budget
                | InlineSkip::Size
                | InlineSkip::Cost
                | InlineSkip::UnknownCost
                | InlineSkip::DataConflict
                | InlineSkip::PrivateReference
        )
    }) || graph.ids().any(|node| {
        if graph.op(node) != Opcode::Call {
            return false;
        }
        let symbol = &graph.callees[graph.payload(node) as usize].symbol;
        provider.graphs.bodies.contains_key(symbol)
            && !provider.graphs.no_inline.contains(symbol)
            && !provider.ready.contains(symbol)
    })
}

fn run(
    graphs: &mut CrateGraphs,
    roots: &BTreeSet<String>,
    limit: usize,
) -> Result<report::Summary, CompileError> {
    let policy = inline_cost::policy().map_err(|message| CompileError {
        symbol: "crate".into(),
        message,
    })?;
    let limits = inline_cost::limits().map_err(|message| CompileError {
        symbol: "crate".into(),
        message,
    })?;
    run_with_configuration(graphs, roots, limit, policy, &limits)
}

fn run_with_configuration(
    graphs: &mut CrateGraphs,
    roots: &BTreeSet<String>,
    limit: usize,
    policy: kfrc_son::InlineCostPolicy,
    limits: &inline_cost::Limits,
) -> Result<report::Summary, CompileError> {
    let reachable = reachable(graphs, roots);
    let order = schedule::postorder(&graphs.bodies, &reachable);
    let mut summary =
        report::Summary::new(graphs.bodies.len(), reachable.len(), order.len(), limit);
    let mut work = Work { limit, used: 0 };
    let mut ready = BTreeSet::new();
    for component in order {
        for symbol in component.members {
            let source = &graphs.bodies[&symbol];
            let mut row = report::Row::new(source);
            if component.recursive {
                row.outcome = "recursive";
                summary.record(row);
                continue;
            }
            if row.source_calls == 0 {
                ready.insert(symbol.clone());
                graphs.inlining_complete.insert(symbol);
                row.emission_inline_complete = true;
                summary.record(row);
                continue;
            }
            let provider = PreparedGraphs {
                graphs,
                ready: &ready,
            };
            let attempt = prepare(source, &provider, &mut work, policy, limits)?;
            row.identity_calls_folded = attempt.identity_calls;
            row.calls_inlined = attempt.inlined.calls_inlined;
            row.record_skips(&attempt.inlined);
            let Some(candidate) = attempt.graph else {
                row.outcome = "work_budget";
                summary.record(row);
                continue;
            };
            row.candidate(&candidate);
            if candidate.symbol != source.symbol || candidate.abi != source.abi {
                return Err(CompileError {
                    symbol: symbol.clone(),
                    message: "bottom-up preparation changed its function boundary".into(),
                });
            }
            let changed = attempt.identity_calls + attempt.inlined.calls_inlined != 0;
            row.emission_inline_complete =
                !emission_needs_retry(&candidate, &provider, &attempt.inlined);
            if changed {
                row.outcome = "published";
                row.final_nodes = row.candidate_nodes;
                row.final_calls = row.candidate_calls;
                graphs.bodies.insert(symbol.clone(), candidate);
            }
            ready.insert(symbol.clone());
            if row.emission_inline_complete {
                graphs.inlining_complete.insert(symbol);
            }
            summary.record(row);
        }
    }
    graphs.collect_references();
    summary.work_used = work.used;
    summary.emit();
    Ok(summary)
}

#[cfg(test)]
mod tests;
