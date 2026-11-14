//! Guarded constant returns from exact internal definitions. The remaining
//! branch keeps the original call; every logical argument is still observed
//! at its original boundary before either branch can execute.
mod analysis;
mod rewrite;

use super::{InlineError, InlineSkip, types::TypeImport};
use crate::verify::verify_pipeline_step;
use crate::{ComparePredicate, FunctionGraph, GraphLevel, NodeId, Opcode, ValueType};
use kfrc_body::{FunctionAbi, RecordedTypes, TypeOracle, TypeShape};
use std::collections::{BTreeMap, BTreeSet};

const MAX_SOURCE_NODES: usize = 512;
const MAX_PATH_STEPS: usize = 64;
const MAX_FIELD_DEPTH: usize = 2;
const MAX_CALLS_SPLIT: usize = 64;

#[derive(Clone, Debug, Default)]
pub struct ConditionalReturnStats {
    pub calls_examined: usize,
    pub calls_split: usize,
    pub summary_nodes_visited: usize,
    pub skipped: Vec<(String, InlineSkip)>,
    /// Stable until compaction; consumers of the original call projections.
    pub dirty_nodes: Vec<NodeId>,
    pub argument_slots: Vec<NodeId>,
}

/// A temporary table for one signature-stable crate wave. It is deliberately
/// separate from memory attributes: no-write alone never permits skipping a
/// call. Rebuild this table after changing definitions or logical signatures.
#[derive(Clone, Debug, Default)]
pub struct ConditionalReturns {
    summaries: BTreeMap<String, Summary>,
}

#[derive(Clone, Debug)]
struct Selector {
    parameter: usize,
    fields: Vec<u32>,
}

#[derive(Clone, Debug)]
struct Summary {
    abi: FunctionAbi,
    oracle: RecordedTypes,
    parameters: ValueType,
    boolean: ValueType,
    lhs: Selector,
    rhs: Selector,
    /// The summarized constant is returned when this comparison is true.
    predicate: ComparePredicate,
    result: bool,
    nodes_visited: usize,
}

impl ConditionalReturns {
    pub fn infer_exact(bodies: &BTreeMap<String, FunctionGraph>, exact: &BTreeSet<String>) -> Self {
        let summaries = bodies
            .iter()
            .filter(|(symbol, graph)| {
                exact.contains(*symbol)
                    && graph.symbol == **symbol
                    && graph.level == GraphLevel::Specialized
                    && !graph.abi.c_variadic
                    && graph.start.index() < graph.len()
                    && graph.len() <= MAX_SOURCE_NODES
            })
            .filter_map(|(symbol, graph)| {
                analysis::summarize(graph).map(|summary| (symbol.clone(), summary))
            })
            .collect();
        Self { summaries }
    }

    /// Cheap prefilter for callers that would otherwise be cloned for a pass.
    pub fn has_candidates(&self, graph: &FunctionGraph) -> bool {
        if self.summaries.is_empty() {
            return false;
        }
        graph.ids().any(|node| {
            graph.op(node) == Opcode::Call
                && graph
                    .callees
                    .get(graph.payload(node) as usize)
                    .is_some_and(|target| self.summaries.contains_key(&target.symbol))
        })
    }

    /// A single snapshot of original calls prevents recursive expansion of the
    /// fallback calls. All edits are committed together after verification.
    pub fn fold_calls(
        &self,
        graph: &mut FunctionGraph,
    ) -> Result<ConditionalReturnStats, InlineError> {
        if graph.level != GraphLevel::Specialized {
            return Err(InlineError::NotSpecialized);
        }
        if !self.has_candidates(graph) {
            return Ok(ConditionalReturnStats::default());
        }
        verify_pipeline_step(graph).map_err(InlineError::Verification)?;
        let calls: Vec<_> = graph
            .ids()
            .filter(|&node| graph.op(node) == Opcode::Call)
            .collect();
        let mut transaction: Option<FunctionGraph> = None;
        let mut stats = ConditionalReturnStats::default();
        let mut visited = BTreeSet::new();
        for call in calls {
            let current = transaction.as_ref().unwrap_or(graph);
            let target = &current.callees[current.payload(call) as usize];
            stats.calls_examined += 1;
            let Some(summary) = self.summaries.get(&target.symbol) else {
                continue;
            };
            if visited.insert(target.symbol.clone()) {
                stats.summary_nodes_visited += summary.nodes_visited;
            }
            // Prove literal selectors in the immutable input snapshot. Earlier
            // splits introduce Regions, which must not invalidate a later
            // original conversion's already-established execution certificate.
            let literals = rewrite::literal_selectors(graph, call, summary);
            let reason = if stats.calls_split == MAX_CALLS_SPLIT {
                Some(InlineSkip::Budget)
            } else if target.abi != summary.abi {
                Some(InlineSkip::AbiMismatch)
            } else if literals.iter().all(Option::is_none) {
                // Avoid adding a duplicate comparison to arbitrary dynamic
                // equality calls. Literal dispatch is the initial consumer.
                Some(InlineSkip::Unsupported)
            } else {
                None
            };
            if let Some(reason) = reason {
                stats.skipped.push((target.symbol.clone(), reason));
                continue;
            }
            let oracle = match boundary(current, call, summary) {
                Ok(oracle) => oracle,
                Err(reason) => {
                    stats.skipped.push((target.symbol.clone(), reason));
                    continue;
                }
            };
            let candidate = transaction.get_or_insert_with(|| graph.clone());
            candidate.oracle = oracle;
            rewrite::split(candidate, call, summary, literals, &mut stats);
        }
        if let Some(mut candidate) = transaction {
            candidate.resolve_edges();
            verify_pipeline_step(&mut candidate).map_err(InlineError::Verification)?;
            for nodes in [&mut stats.dirty_nodes, &mut stats.argument_slots] {
                for node in nodes.iter_mut() {
                    *node = candidate.resolve(*node);
                }
                nodes.retain(|&node| !node.is_none() && !candidate.is_dead(node));
                nodes.sort_unstable();
                nodes.dedup();
            }
            *graph = candidate;
        }
        Ok(stats)
    }
}

fn boundary(
    graph: &FunctionGraph,
    call: NodeId,
    summary: &Summary,
) -> Result<RecordedTypes, InlineSkip> {
    let mut oracle = graph.oracle.clone();
    let mut import = TypeImport::new(&summary.oracle, &mut oracle);
    let parameters = import.value_type(&summary.parameters)?;
    let boolean = import.value_type(&summary.boolean)?;
    import.finish()?;
    let ValueType::Tuple(parameters) = parameters else {
        return Err(InlineSkip::TypeConflict);
    };
    let args = &graph.inputs(call)[1..];
    let expected = ValueType::Tuple(vec![ValueType::Control, ValueType::Effect, boolean]);
    if args.len() + 2 != parameters.len()
        || graph.value_type(call) != &expected
        || args
            .iter()
            .zip(&parameters[2..])
            .any(|(&arg, ty)| graph.value_type(graph.resolve(arg)) != ty)
    {
        return Err(InlineSkip::TypeConflict);
    }
    // Imported semantic facts must also support every selector. In particular,
    // unknown metadata cannot be treated as the second word of a pointer.
    let selector_type = |selector: &Selector| {
        let ValueType::Rust(ty) = parameters.get(selector.parameter + 2)? else {
            return None;
        };
        let mut ty = *ty;
        for &field in &selector.fields {
            ty = oracle.semantic().field_type(ty, None, field)?;
        }
        matches!(oracle.shape(ty), TypeShape::Int { .. }).then_some(ty)
    };
    if selector_type(&summary.lhs).is_none()
        || selector_type(&summary.lhs) != selector_type(&summary.rhs)
    {
        return Err(InlineSkip::TypeConflict);
    }
    Ok(oracle)
}
