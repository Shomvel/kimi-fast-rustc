//! Shared inlining driver. Preparation has one fuel account for its whole tree.
mod priority;
mod temperature;

use crate::analysis::cfg::ControlFlow;
use crate::analysis::cost::GraphCost;
use crate::verify::verify_pipeline_step;
use crate::{FunctionGraph, GraphLevel, NodeId, Opcode};
use kfrc_body::fx_hash::{HashMap, HashSet};
use std::cmp::Reverse;
use std::collections::BinaryHeap;

use super::arguments::Arguments;
use super::cost::Costs;
use super::loop_constants::LoopConstants;
use super::plan::{self, plan};
use super::splice::splice;
use super::{
    GraphProvider, InlineBudget, InlineCostPolicy, InlineDecision, InlineError, InlineSkip,
    InlineStats,
};

#[derive(Clone, Copy)]
pub(super) enum Frame {
    Emission,
    Preparation { depth: usize },
}

impl Frame {
    pub fn is_preparation(self) -> bool {
        matches!(self, Self::Preparation { .. })
    }

    fn first_depth(self) -> usize {
        match self {
            Self::Emission => 0,
            Self::Preparation { depth } => depth + 1,
        }
    }

    fn limit(self, budget: InlineBudget) -> usize {
        if self.is_preparation() {
            super::arguments::MAX_DEPTH
        } else {
            budget.max_depth
        }
    }

    fn preparation_depth(self, call_depth: usize) -> usize {
        if self.is_preparation() { call_depth } else { 0 }
    }

    fn free(
        self,
        budget: InlineBudget,
        size: usize,
        estimate: Option<GraphCost>,
        call_units: Option<u64>,
        policy: InlineCostPolicy,
    ) -> bool {
        // Legacy and the seed model retain the old preparation contract.
        // Residual pricing uses the same currency in both frames.
        if self.is_preparation() && !matches!(policy, InlineCostPolicy::Residual { .. }) {
            return size <= budget.free_nodes;
        }
        match policy {
            InlineCostPolicy::CodeSize { free_units, .. } => estimate
                .and_then(|cost| cost.code_size.total())
                .is_some_and(|cost| cost <= free_units),
            InlineCostPolicy::Residual { .. } => estimate
                .and_then(|cost| cost.code_size.total())
                .zip(call_units)
                .is_some_and(|(cost, call)| cost <= call),
            _ => size <= budget.free_nodes,
        }
    }
}

pub(super) fn run(
    caller: &mut FunctionGraph,
    provider: &(impl GraphProvider + ?Sized),
    budget: InlineBudget,
    arguments: &mut Arguments,
    costs: &mut Costs<'_>,
    frame: Frame,
) -> Result<InlineStats, InlineError> {
    if caller.level != GraphLevel::Specialized {
        return Err(InlineError::NotSpecialized);
    }
    verify_pipeline_step(caller).map_err(InlineError::Verification)?;
    let mut stats = InlineStats::default();
    // One transaction per function, not a complete caller copy per call site.
    let original = caller;
    let mut candidate = None;
    let mut source_checks = HashMap::default();
    let mut recursion = HashMap::default();
    let mut callees = HashMap::default();
    let mut imports: HashMap<(String, usize), plan::ImportedTypes> = HashMap::default();
    let mut seams = None;
    let mut pricing = temperature::Pricing::new(costs.policy, frame.is_preparation(), original);
    let mut sequence = 0usize;
    let mut pending: Pending = BinaryHeap::new();
    for call in original.ids().filter(|&n| original.op(n) == Opcode::Call) {
        pending.push(Reverse((
            !provider.always_inline(&original.callees[original.payload(call) as usize].symbol),
            pricing.temperature(call) != super::InlineTemperature::Hot,
            costs.priority(original, call, provider),
            sequence,
            call,
            frame.first_depth(),
        )));
        sequence += 1;
    }
    if frame.is_preparation() {
        prioritize(original, &mut pending, arguments);
    }
    let mut costed_calls = 0usize;
    let mut loop_constants = LoopConstants::new(
        if frame.is_preparation() || budget.copy_anonymous_constants {
            0
        } else {
            budget.max_loop_constant_bytes
        },
        budget.max_nodes.saturating_mul(8),
    );
    while let Some(Reverse((_, _, _, _, call, depth))) = pending.pop() {
        let caller = candidate.as_ref().unwrap_or(&*original);
        if caller.is_dead(call) {
            continue;
        }
        if frame.is_preparation() {
            stats.nodes_visited += caller.live_count();
            if !ControlFlow::new(caller).contains(caller.control(call)) {
                continue;
            }
        }
        let policy = pricing.policy(costs.policy, call);
        let temperature = pricing.temperature(call);
        let target = caller.callees[caller.payload(call) as usize].clone();
        let always = provider.always_inline(&target.symbol);
        let original_size = provider
            .preparation_graph(&target.symbol)
            .map_or(0, FunctionGraph::live_count);
        let mut size = original_size;
        let mut estimate = costs.estimate(&target.symbol, provider);
        let call_units = costs.call_units(caller, call);
        let sole = depth == 0 && provider.sole_call_site(&target.symbol);
        let mut free = frame.free(budget, size, estimate, call_units, policy);
        let decision = (policy != InlineCostPolicy::Legacy || frame.is_preparation()).then(|| {
            let index = stats.decisions.len();
            stats.decisions.push(InlineDecision {
                call,
                symbol: target.symbol.clone(),
                depth,
                source_nodes: size,
                specialized_nodes: None,
                budget,
                nodes_added_before: stats.nodes_added,
                costed_calls_before: costed_calls,
                model: costs.model_name(),
                policy,
                temperature,
                hot_growth_remaining: pricing.hot_growth_remaining(),
                hot_growth_charged: 0,
                estimate,
                call_units,
                sole_call_site: sole,
                free,
                rejection: None,
            });
            index
        });
        if depth >= frame.limit(budget) && !matches!(policy, InlineCostPolicy::Residual { .. }) {
            stats.reject(target.symbol, InlineSkip::Budget, decision);
            continue;
        }
        let Some(source) = provider.preparation_graph(&target.symbol) else {
            stats.reject(target.symbol, InlineSkip::Missing, decision);
            continue;
        };
        let could_prepare = Arguments::supports_source(source)
            && (frame.is_preparation()
                || (original_size > budget.max_callee_nodes && !sole)
                || matches!(policy, InlineCostPolicy::Residual { .. })
                || provider.graph(&target.symbol).is_none()
                || source
                    .data_objects
                    .iter()
                    .any(|data| data.binding == kfrc_body::DataBinding::Local));
        if !could_prepare {
            // Preserve cheap rejection for sources that cannot be refined.
            // When preparation is possible, a cached/smaller candidate can
            // still become free or fit the remaining emitted-growth budget.
            if !free && costed_calls >= budget.max_calls {
                stats.reject(target.symbol, InlineSkip::Budget, decision);
                continue;
            }
            if (!policy.uses_size()
                || (frame.is_preparation() && !matches!(policy, InlineCostPolicy::Residual { .. })))
                && size > budget.max_callee_nodes
                && !sole
                && !always
            {
                stats.reject(target.symbol, InlineSkip::Size, decision);
                continue;
            }
            if source.live_count() > budget.max_nodes.saturating_sub(stats.nodes_added) {
                stats.reject(target.symbol, InlineSkip::Budget, decision);
                continue;
            }
        }
        let source_eligible = if let Some(&eligible) = source_checks.get(&target.symbol) {
            eligible
        } else {
            stats.nodes_visited += source.live_count();
            let eligible = source.level == GraphLevel::Specialized && eligible(source);
            if eligible {
                crate::verify::check_pipeline_step(source).map_err(InlineError::Verification)?;
            }
            source_checks.insert(target.symbol.clone(), eligible);
            eligible
        };
        if !source_eligible {
            stats.reject(target.symbol, InlineSkip::Unsupported, decision);
            continue;
        }
        if source.abi != target.abi {
            stats.reject(target.symbol, InlineSkip::AbiMismatch, decision);
            continue;
        }
        // A large body can become an ordinary small inline after literal
        // parameters discard its checks/cold paths. This never widens the
        // accepted size limit or permits copying identity-bearing live data.
        let prepared = if could_prepare {
            arguments.prepare(
                caller,
                call,
                source,
                provider,
                costs,
                frame.preparation_depth(depth),
            )?
        } else {
            None
        };
        let specialization = prepared.as_ref().map_or(0, |prepared| prepared.index);
        let cached = prepared.as_ref().is_some_and(|prepared| prepared.cached);
        let source = prepared
            .as_ref()
            .map_or(source, |prepared| prepared.source.as_ref());
        if specialization != 0 {
            size = source.live_count();
            estimate = costs.estimate_source(&target.symbol, specialization, source);
            free = frame.free(budget, size, estimate, call_units, policy);
            if let Some(index) = decision {
                stats.decisions[index].specialized_nodes = Some(size);
                stats.decisions[index].estimate = estimate;
                stats.decisions[index].free = free;
            }
        }
        if matches!(policy, InlineCostPolicy::Residual { .. })
            && depth >= frame.limit(budget)
            && (frame.is_preparation() || !free)
        {
            stats.reject(target.symbol, InlineSkip::Budget, decision);
            continue;
        }
        if !free && costed_calls >= budget.max_calls {
            stats.reject(target.symbol, InlineSkip::Budget, decision);
            continue;
        }
        // Observe follows Legacy, including call-site preparation.
        if (!policy.uses_size()
            || (frame.is_preparation() && !matches!(policy, InlineCostPolicy::Residual { .. })))
            && size > budget.max_callee_nodes
            && !sole
            && !always
        {
            stats.reject(target.symbol, InlineSkip::Size, decision);
            continue;
        }
        if source.live_count() > budget.max_nodes.saturating_sub(stats.nodes_added) {
            stats.reject(target.symbol, InlineSkip::Budget, decision);
            continue;
        }
        if *recursion.entry(target.symbol.clone()).or_insert_with(|| {
            recursive(
                provider,
                &target.symbol,
                &caller.symbol,
                &mut callees,
                &mut stats.nodes_visited,
            )
        }) {
            stats.reject(target.symbol, InlineSkip::Recursive, decision);
            continue;
        }
        let maximum = match policy {
            InlineCostPolicy::CodeSize { max_units, .. } => Some(Some(max_units)),
            InlineCostPolicy::Residual { max_growth } => {
                Some(call_units.and_then(|call| call.checked_add(max_growth)))
            }
            _ => None,
        };
        if let Some(maximum) = maximum
            && !always
        {
            match (estimate.and_then(|cost| cost.code_size.total()), maximum) {
                (Some(units), Some(maximum)) if units > maximum && !sole => {
                    stats.reject(target.symbol, InlineSkip::Cost, decision);
                    continue;
                }
                (Some(_), Some(_)) => {}
                _ => {
                    stats.reject(target.symbol, InlineSkip::UnknownCost, decision);
                    continue;
                }
            }
        }
        let before = caller.len();
        let transaction = if frame.is_preparation() && candidate.is_none() {
            before
        } else {
            0
        };
        let mut remaining = budget.max_nodes.saturating_sub(stats.nodes_added);
        if frame.is_preparation() {
            if !arguments.helpers_available(free) || transaction > arguments.remaining() {
                stats.reject(target.symbol, InlineSkip::Budget, decision);
                continue;
            }
            remaining = remaining.min(arguments.remaining() - transaction);
        }
        let import_key = (target.symbol.clone(), specialization);
        let planned = plan(
            caller,
            call,
            source,
            remaining,
            budget.copy_anonymous_constants,
            None,
            imports.get(&import_key),
            provider,
        );
        let planned = match planned {
            Err(InlineSkip::DataConflict) => match loop_constants.remaining(original, call) {
                Some(bytes) => plan(
                    caller,
                    call,
                    source,
                    remaining,
                    false,
                    Some(bytes),
                    imports.get(&import_key),
                    provider,
                ),
                None => Err(InlineSkip::DataConflict),
            },
            result => result,
        };
        let plan = match planned {
            Ok(plan) => plan,
            Err(reason) => {
                stats.reject(target.symbol, reason, decision);
                continue;
            }
        };
        let constant_bytes_imported = plan.constant_bytes_imported;
        stats.nodes_visited += plan.ids.len();
        // Every successful plan is spliced, so its import is now in the oracle.
        imports
            .entry(import_key.clone())
            .or_insert_with(|| plan.imported.clone());
        if frame.is_preparation() {
            arguments.transaction(transaction);
        }
        let caller = candidate.get_or_insert_with(|| original.clone());
        // Preserve the index across ordinary splices. The scratch preparation
        // path already alternates its own control/fact cleanup with splicing.
        if !frame.is_preparation() && seams.is_none() {
            seams = Some(crate::transforms::scalar::seam::Worklist::new(caller));
        }
        let spliced = splice(caller, call, source, plan);
        if !always && !sole {
            let charge = pricing.charge(costs.policy, call, estimate, call_units);
            stats.hot_growth_spent += charge;
            if let Some(index) = decision {
                stats.decisions[index].hot_growth_charged = charge;
            }
        }
        pricing.inherit(temperature, &import_key, source, &spliced.nested);
        let control_dirty = if let Some(seams) = &mut seams {
            let available = budget
                .max_nodes
                .saturating_sub(stats.nodes_added + caller.len() - before);
            let cleaned = seams.run_bounded(caller, &spliced.dirty, available);
            stats.seam_nodes_indexed += cleaned.nodes_indexed;
            stats.seam_node_visits += cleaned.nodes_visited;
            stats.seam_allocation_bailouts += usize::from(cleaned.allocation_limited);
            stats.nodes_visited += cleaned.nodes_indexed + cleaned.nodes_visited;
            cleaned.control_dirty
        } else {
            false
        };
        if spliced.diverges || control_dirty {
            stats.nodes_visited += caller.live_count();
            crate::transforms::control::simplify::prune(caller)
                .map_err(crate::PipelineError::Control)
                .map_err(InlineError::Specialization)?;
            stats.seam_control_cleanups += 1;
            // Region/Phi edits can touch users outside the scalar seam.
            seams = None;
        }
        if !budget.copy_anonymous_constants {
            loop_constants.imported(constant_bytes_imported);
        }
        stats.constant_bytes_imported += constant_bytes_imported;
        let allocated = caller.len() - before;
        if frame.is_preparation() {
            debug_assert!(allocated <= remaining);
            arguments.spliced(allocated, spliced.cloned_nodes, free, cached);
        }
        stats.calls_inlined += 1;
        costed_calls += usize::from(!free);
        stats.nodes_added += allocated;
        stats.cloned_nodes += spliced.cloned_nodes;
        stats.dirty_nodes.extend(spliced.dirty);
        stats.argument_slots.extend(spliced.argument_slots);
        if frame.is_preparation() && !arguments.clean(caller)? {
            break;
        }
        // A prepared source already visited its surviving calls with this
        // tree's depth/fuel. Re-enqueuing them here would shorten their path
        // when the source is spliced back through several preparation frames.
        // Emission retains its separate, ordinary depth-four queue.
        if !frame.is_preparation() || specialization == 0 {
            for (_, node) in spliced.nested {
                if caller.is_dead(node) {
                    continue;
                }
                loop_constants.inherit(call, node);
                pending.push(Reverse((
                    !provider.always_inline(&caller.callees[caller.payload(node) as usize].symbol),
                    pricing.temperature(node) != super::InlineTemperature::Hot,
                    costs.priority(caller, node, provider),
                    sequence,
                    node,
                    depth + 1,
                )));
                sequence += 1;
            }
        }
        if frame.is_preparation() && !pending.is_empty() {
            // Reprice only already queued calls after cleanup exposes facts.
            // This never adds residual calls from a prepared source.
            prioritize(caller, &mut pending, arguments);
        }
    }
    stats.loop_constant_work = loop_constants.work;
    stats.temperature_work = pricing.work;
    stats.temperature_budget_bailouts = pricing.bailouts;
    stats.nodes_visited += pricing.work;
    if frame.is_preparation() {
        arguments.record_driver(&original.symbol, &mut stats);
    }
    let Some(mut candidate) = candidate else {
        return Ok(stats);
    };
    let caller = &mut candidate;
    caller.resolve_edges();
    verify_pipeline_step(caller).map_err(InlineError::Verification)?;
    for nodes in [&mut stats.dirty_nodes, &mut stats.argument_slots] {
        for node in nodes.iter_mut() {
            *node = caller.resolve(*node);
        }
        nodes.retain(|node| !node.is_none() && !caller.is_dead(*node));
        nodes.sort_unstable();
        nodes.dedup();
    }
    *original = candidate;
    Ok(stats)
}

// Strong intent comes first. Emission then prioritizes hot sites;
// Preparation prioritizes branch drivers within that same strong intent.
type Pending = BinaryHeap<Reverse<(bool, bool, u64, usize, NodeId, usize)>>;

fn prioritize(graph: &FunctionGraph, pending: &mut Pending, arguments: &mut Arguments) {
    let work = priority::branch_drivers(graph, arguments.priority_remaining());
    arguments.priority_work(work.visits, work.bailed_out);
    *pending = std::mem::take(pending)
        .into_iter()
        .map(|Reverse((ordinary, _, size, sequence, call, depth))| {
            Reverse((
                ordinary,
                !work.calls.contains(&call),
                size,
                sequence,
                call,
                depth,
            ))
        })
        .collect();
}

fn recursive(
    provider: &(impl GraphProvider + ?Sized),
    target: &str,
    caller: &str,
    callees: &mut HashMap<String, Vec<String>>,
    visited: &mut usize,
) -> bool {
    if target == caller {
        return true;
    }
    let mut seen = HashSet::default();
    let mut pending = vec![target.to_owned()];
    while let Some(symbol) = pending.pop() {
        if !seen.insert(symbol.clone()) {
            continue;
        }
        let adjacent = callees.entry(symbol.clone()).or_insert_with(|| {
            let Some(graph) = provider.preparation_graph(&symbol) else {
                return Vec::new();
            };
            *visited += graph.live_count();
            graph
                .ids()
                .filter(|&n| graph.op(n) == Opcode::Call)
                .map(|n| graph.callees[graph.payload(n) as usize].symbol.clone())
                .collect()
        });
        for callee in adjacent {
            if callee == caller || callee == target {
                return true;
            }
            pending.push(callee.clone());
        }
    }
    false
}

fn eligible(source: &FunctionGraph) -> bool {
    source.ids().all(|n| {
        matches!(
            source.op(n),
            Opcode::Start
                | Opcode::Projection
                | Opcode::Region
                | Opcode::Jump
                | Opcode::If
                | Opcode::VariantSwitch
                | Opcode::VariantWitness
                | Opcode::Assume
                | Opcode::Phi
                | Opcode::Return
                | Opcode::Unreachable
                | Opcode::Call
                | Opcode::CallIndirect
                | Opcode::Opaque
                | Opcode::RequireDefined
                | Opcode::Const
                | Opcode::Undefined
                | Opcode::Aggregate
                | Opcode::Repeat
                | Opcode::Field
                | Opcode::InsertField
                | Opcode::MakeVariant
                | Opcode::VariantOf
                | Opcode::VariantField
                | Opcode::WidePtr
                | Opcode::Discriminant
                | Opcode::Add
                | Opcode::Sub
                | Opcode::Mul
                | Opcode::DivSigned
                | Opcode::DivUnsigned
                | Opcode::RemSigned
                | Opcode::RemUnsigned
                | Opcode::BitAnd
                | Opcode::BitOr
                | Opcode::BitXor
                | Opcode::ShiftLeft
                | Opcode::ShiftRightArithmetic
                | Opcode::ShiftRightLogical
                | Opcode::Negate
                | Opcode::BitNot
                | Opcode::Compare
                | Opcode::IntCast
                | Opcode::MulOverflow
                | Opcode::BitCount
                | Opcode::ByteSwap
                | Opcode::BitReverse
                | Opcode::FunnelShift
                | Opcode::SaturatingAdd
                | Opcode::SaturatingSub
                | Opcode::SymbolAddress
                | Opcode::PointerCast
                | Opcode::LocalSlot
                | Opcode::FieldAddr
                | Opcode::VariantFieldAddr
                | Opcode::IndexAddr
                | Opcode::LoadValue
                | Opcode::StoreValue
                | Opcode::SetVariant
                | Opcode::SlotLive
                | Opcode::SlotDead
                | Opcode::CopyNonOverlapping
                | Opcode::ConstRepr
                | Opcode::TransmuteValue
        )
    })
}
