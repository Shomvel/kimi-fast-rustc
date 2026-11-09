//! Budgeted same-crate inlining of verified specialized value graphs.
//!
//! The producer must run the original collector's required-constant and
//! mentioned-item diagnostics before calling this pass and independently prove
//! emission reachability. This pass does not replace that proof or definition-
//! level R specialization. Missing or ineligible callees stay calls.
mod identity;
mod plan;
mod splice;
mod types;

pub use identity::{IdentityFoldStats, fold_identity_calls};

use crate::{FunctionGraph, GraphLevel, NodeId, Opcode, VerifyError, verify_graph};
use plan::plan;
use splice::splice;
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};

pub trait GraphProvider {
    fn graph(&self, symbol: &str) -> Option<&FunctionGraph>;
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

#[derive(Clone, Copy, Debug)]
pub struct InlineBudget {
    pub max_depth: usize,
    pub max_nodes: usize,
    pub max_calls: usize,
}
impl Default for InlineBudget {
    fn default() -> Self {
        Self {
            max_depth: 4,
            max_nodes: 2000,
            max_calls: 32,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InlineSkip {
    Missing,
    Recursive,
    Budget,
    Unsupported,
    TypeConflict,
    AbiMismatch,
    DataConflict,
}
#[derive(Clone, Debug, Default)]
pub struct InlineStats {
    pub calls_inlined: usize,
    pub nodes_added: usize,
    pub skipped: Vec<(String, InlineSkip)>,
    /// Nodes inspected by eligibility, recursion discovery and splice planning.
    pub nodes_visited: usize,
    /// Source graph nodes copied, excluding call-boundary and lifetime nodes.
    pub cloned_nodes: usize,
    /// Stable until the caller compacts: seam users, returns and lifetime nodes.
    pub dirty_nodes: Vec<NodeId>,
    /// Caller LocalSlot roots passed to an erased call (W06/F08).
    pub argument_slots: Vec<NodeId>,
}
#[derive(Debug)]
pub enum InlineError {
    NotSpecialized,
    Verification(VerifyError),
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
    if caller.level != GraphLevel::Specialized {
        return Err(InlineError::NotSpecialized);
    }
    verify_graph(caller).map_err(InlineError::Verification)?;
    let mut stats = InlineStats::default();
    // One transaction per function, not a complete caller copy per call site.
    let original = caller;
    let mut candidate = None;
    let mut source_checks = HashMap::new();
    let mut recursion = HashMap::new();
    let mut callees = HashMap::new();
    let mut pending: VecDeque<_> = original
        .ids()
        .filter(|&n| original.op(n) == Opcode::Call)
        .map(|n| (n, 0usize))
        .collect();
    while let Some((call, depth)) = pending.pop_front() {
        let caller = candidate.as_ref().unwrap_or(&*original);
        if caller.is_dead(call) {
            continue;
        }
        let target = caller.callees[caller.payload(call) as usize].clone();
        if depth >= budget.max_depth || stats.calls_inlined >= budget.max_calls {
            stats.skipped.push((target.symbol, InlineSkip::Budget));
            continue;
        }
        let Some(source) = provider.graph(&target.symbol) else {
            stats.skipped.push((target.symbol, InlineSkip::Missing));
            continue;
        };
        let source_eligible = if let Some(&eligible) = source_checks.get(&target.symbol) {
            eligible
        } else {
            stats.nodes_visited += source.ids().count();
            let eligible = source.level == GraphLevel::Specialized && eligible(source);
            if eligible {
                crate::verify::check(source).map_err(InlineError::Verification)?;
            }
            source_checks.insert(target.symbol.clone(), eligible);
            eligible
        };
        if !source_eligible {
            stats.skipped.push((target.symbol, InlineSkip::Unsupported));
            continue;
        }
        if source.ids().count() > budget.max_nodes.saturating_sub(stats.nodes_added) {
            stats.skipped.push((target.symbol, InlineSkip::Budget));
            continue;
        }
        if source.abi != target.abi {
            stats.skipped.push((target.symbol, InlineSkip::AbiMismatch));
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
            stats.skipped.push((target.symbol, InlineSkip::Recursive));
            continue;
        }
        let before = caller.len();
        let plan = match plan(
            caller,
            call,
            source,
            budget.max_nodes.saturating_sub(stats.nodes_added),
        ) {
            Ok(plan) => plan,
            Err(reason) => {
                stats.skipped.push((target.symbol, reason));
                continue;
            }
        };
        stats.nodes_visited += plan.ids.len();
        let caller = candidate.get_or_insert_with(|| original.clone());
        let spliced = splice(caller, call, source, plan);
        stats.calls_inlined += 1;
        stats.nodes_added += caller.len() - before;
        stats.cloned_nodes += spliced.cloned_nodes;
        stats.dirty_nodes.extend(spliced.dirty);
        stats.argument_slots.extend(spliced.argument_slots);
        pending.extend(spliced.nested.into_iter().map(|node| (node, depth + 1)));
    }
    let Some(mut candidate) = candidate else {
        return Ok(stats);
    };
    let caller = &mut candidate;
    caller.resolve_edges();
    verify_graph(caller).map_err(InlineError::Verification)?;
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
    let mut seen = HashSet::new();
    let mut pending = vec![target.to_owned()];
    while let Some(symbol) = pending.pop() {
        if !seen.insert(symbol.clone()) {
            continue;
        }
        let adjacent = callees.entry(symbol.clone()).or_insert_with(|| {
            let Some(graph) = provider.graph(&symbol) else {
                return Vec::new();
            };
            *visited += graph.ids().count();
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
    source.ids().any(|n| source.op(n) == Opcode::Return)
        && source.ids().all(|n| {
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
