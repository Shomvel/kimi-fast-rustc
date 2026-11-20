//! Fold literal branches, trim merge slots, then discard unreachable graphs.
//!
//! This pass does not speculate values or infer conditions from poison. Fixed
//! operations on retained control paths, including unused Div/Rem, are roots.

use kfrc_body::TypeShape;
use kfrc_body::fx_hash::{HashMap, HashSet};

use crate::analysis::cfg::{ControlFlow, is_anchored_value, is_control_flow};
use crate::transforms::scalar::instcombine;
use crate::transforms::utils::cfg::{CfgEditError, CfgEditor};
use crate::{FunctionGraph, NodeId, Opcode, ValueType};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ControlStats {
    pub branches_folded: usize,
    pub predecessor_edges_removed: usize,
    pub nodes_removed: usize,
    pub phis_folded: usize,
}

/// Forward every Phi whose incoming values, ignoring the Phi itself, are one
/// value of the same type. Inlined merges otherwise leave chains of such Phis
/// (a slice pointer re-merged at every join), so equal SSA values appear as
/// distinct nodes to later pattern matching.
fn fold_redundant_phis(graph: &mut FunctionGraph) -> usize {
    let phis: Vec<_> = graph
        .ids()
        .filter(|node| graph.op(*node) == Opcode::Phi)
        .collect();
    // Every fold starts at a Phi that is redundant now; later folds only
    // revisit users of folded Phis. Without one, the worklist changes nothing.
    if !phis.iter().any(|&phi| redundant_value(graph, phi).is_some()) {
        return 0;
    }
    let mut phi_users: HashMap<NodeId, Vec<NodeId>> = HashMap::default();
    for &phi in &phis {
        for &input in &graph.inputs(phi)[1..] {
            let input = graph.resolve(input);
            if input != phi && graph.op(input) == Opcode::Phi {
                phi_users.entry(input).or_default().push(phi);
            }
        }
    }
    let mut pending = phis;
    pending.reverse();
    let mut folded = 0;
    while let Some(phi) = pending.pop() {
        if graph.is_dead(phi) {
            continue;
        }
        let Some(value) = redundant_value(graph, phi) else {
            continue;
        };
        graph.replace(phi, value);
        folded += 1;
        if let Some(users) = phi_users.remove(&phi) {
            // Users now observe `value`; they must be revisited if it folds.
            if graph.op(value) == Opcode::Phi {
                phi_users.entry(value).or_default().extend(users.iter().copied());
            }
            pending.extend(users);
        }
    }
    folded
}

/// The one same-typed value all incoming edges carry, ignoring the Phi itself.
fn redundant_value(graph: &FunctionGraph, phi: NodeId) -> Option<NodeId> {
    let mut same = None;
    for &input in &graph.inputs(phi)[1..] {
        let input = graph.resolve(input);
        if input == phi {
            continue;
        }
        match same {
            None => same = Some(input),
            Some(value) if value == input => (),
            Some(_) => return None,
        }
    }
    same.filter(|value| !value.is_none() && graph.value_type(*value) == graph.value_type(phi))
}

/// End every path through an `Assume` of literal `false` with `Unreachable`.
///
/// Reaching such an Assume is undefined behavior, so its continuation never
/// executes. MIR produces one when a branch whose other target is unreachable
/// tests a session constant, e.g. `assume(ub_checks())` guarding an unsafe
/// precondition check once UB checks are off. The path's operations go with
/// it; the sweep in `run_impl` removes the dead nodes and the Region slots
/// they fed.
fn terminate_false_assumptions(graph: &mut FunctionGraph) -> usize {
    let assumptions: Vec<_> = graph
        .ids()
        .filter(|&node| {
            graph.op(node) == Opcode::Assume && {
                let predicate = graph.resolve(graph.input(node, 1));
                graph.op(predicate) == Opcode::Const
                    && graph.rust_type(predicate).is_some_and(|ty| {
                        matches!(graph.oracle.semantic().shape(ty), TypeShape::Bool)
                    })
                    && graph.constant_value(predicate) == 0
            }
        })
        .collect();
    for &assumption in &assumptions {
        let predecessor = graph.resolve(graph.control(assumption));
        graph.add_node(Opcode::Unreachable, predecessor, &[], 0, ValueType::Control);
        // Its projections keep this dead source, so they and their users are
        // unreachable from the new terminal and retire in the sweep.
        graph.remove_node(assumption);
    }
    assumptions.len()
}

/// Select the live edge of every `If` whose other successor immediately
/// reaches Unreachable, as LLVM's SimplifyCFG does for a branch into an
/// `unreachable` block: taking that successor is undefined behavior.
///
/// When the If directly follows a Region and its condition is decided by
/// constant Phi inputs, entering the Region along such an edge already commits
/// to the undefined successor, so that edge's Jump also becomes Unreachable.
/// The next round then folds the branch that chose that edge. Paths with any
/// operation before Unreachable (a diverging call, a store) are kept.
fn fold_unreachable_branches(graph: &mut FunctionGraph, folds: &mut Vec<(NodeId, u32)>) {
    let mut seen: HashSet<NodeId> = folds.iter().map(|&(branch, _)| branch).collect();
    let terminals: Vec<_> = graph
        .ids()
        .filter(|&node| graph.op(node) == Opcode::Unreachable)
        .collect();
    let mut undefined_entries = Vec::new();
    for terminal in terminals {
        let edge = graph.resolve(graph.control(terminal));
        if edge.is_none() || graph.op(edge) != Opcode::Projection {
            continue;
        }
        let branch = graph.resolve(graph.input(edge, 0));
        if graph.op(branch) != Opcode::If || !seen.insert(branch) {
            continue;
        }
        let dead = graph.payload(edge);
        // Projection 0 is taken for a true condition, 1 for false.
        let undefined_condition = u64::from(dead == 0);
        let region = graph.resolve(graph.control(branch));
        if graph.op(region) == Opcode::Region {
            let condition = graph.resolve(graph.input(branch, 0));
            for (slot, &jump) in graph.inputs(region).iter().enumerate() {
                let incoming = |node: NodeId| {
                    let node = graph.resolve(node);
                    match graph.op(node) {
                        Opcode::Const => instcombine::constant(graph, node),
                        Opcode::Phi if graph.resolve(graph.input(node, 0)) == region => {
                            let value = graph.resolve(graph.input(node, slot + 1));
                            (graph.op(value) == Opcode::Const)
                                .then(|| instcombine::constant(graph, value))
                                .flatten()
                        }
                        _ => None,
                    }
                };
                let decided = if graph.op(condition) == Opcode::Phi {
                    incoming(condition)
                } else if instcombine::supports_evaluation(graph.op(condition)) {
                    instcombine::evaluate(graph, condition, incoming)
                } else {
                    None
                };
                let jump = graph.resolve(jump);
                if decided == Some(undefined_condition) && graph.op(jump) == Opcode::Jump {
                    undefined_entries.push(jump);
                }
            }
        }
        folds.push((branch, 1 - dead));
    }
    for jump in undefined_entries {
        let control = graph.resolve(graph.control(jump));
        graph.add_node(Opcode::Unreachable, control, &[], 0, ValueType::Control);
        // The Region slot fed by this Jump is trimmed by the sweep below.
        graph.remove_node(jump);
    }
}

pub fn run(graph: &mut FunctionGraph) -> Result<ControlStats, CfgEditError> {
    run_impl(graph, true, false)
}

/// `run`, then repeatedly fold branches into Unreachable (see
/// `fold_unreachable_branches`). A branch into Unreachable still carries its
/// condition as a fact for condition and variant threading, so only the final
/// cleanup, after those consumers, trades it for shorter control flow.
pub(crate) fn run_final(graph: &mut FunctionGraph) -> Result<ControlStats, CfgEditError> {
    let mut total = run_impl(graph, true, false)?;
    // Each round can expose one more branch level upstream; the bound keeps
    // the cleanup linear in practice.
    for _ in 0..8 {
        let stats = run_impl(graph, true, true)?;
        total.branches_folded += stats.branches_folded;
        total.predecessor_edges_removed += stats.predecessor_edges_removed;
        total.nodes_removed += stats.nodes_removed;
        total.phis_folded += stats.phis_folded;
        if stats.branches_folded == 0 {
            break;
        }
    }
    Ok(total)
}

/// Reachability/Region/Phi repair without creating any new graph records.
/// Inliner growth accounts can be exhausted when this repair is mandatory.
pub(crate) fn prune(graph: &mut FunctionGraph) -> Result<ControlStats, CfgEditError> {
    run_impl(graph, false, false)
}

fn run_impl(
    graph: &mut FunctionGraph,
    fold_diamonds: bool,
    unreachable_branches: bool,
) -> Result<ControlStats, CfgEditError> {
    graph.resolve_edges();
    let before = graph.live_count();
    let mut stats = ControlStats::default();
    if fold_diamonds {
        stats.branches_folded += super::diamonds::run(graph);
        stats.branches_folded += terminate_false_assumptions(graph);
    }
    // Decisions depend only on condition values; folding rewrites control
    // edges, so every decision can be taken before the first fold.
    let mut folds = Vec::new();
    for branch in graph
        .ids()
        .filter(|node| matches!(graph.op(*node), Opcode::If | Opcode::VariantSwitch))
    {
        let Some(&condition) = graph.inputs(branch).first() else {
            continue;
        };
        let condition = graph.resolve(condition);
        let taken = match graph.op(branch) {
            Opcode::VariantSwitch if graph.op(condition) == Opcode::MakeVariant => {
                let cases = &graph.variant_switches[graph.payload(branch) as usize];
                cases
                    .iter()
                    .position(|variant| *variant == graph.payload(condition))
                    .unwrap_or(cases.len()) as u32
            }
            Opcode::If if graph.op(condition) == Opcode::Const => {
                let Some(ty) = graph.rust_type(condition) else {
                    continue;
                };
                if !matches!(graph.oracle.semantic().shape(ty), TypeShape::Bool) {
                    continue;
                }
                match graph.constant_value(condition) {
                    0 => 1,
                    1 => 0,
                    _ => continue,
                }
            }
            _ => continue,
        };
        folds.push((branch, taken));
    }
    if unreachable_branches {
        fold_unreachable_branches(graph, &mut folds);
    }
    // Branch folding only forwards/removes existing nodes, so one immutable
    // projection index remains valid through this loop. Avoid rescanning the
    // entire graph for every selected edge.
    let mut projections: HashMap<(NodeId, u32), Vec<NodeId>> = HashMap::default();
    if !folds.is_empty() {
        for node in graph.ids() {
            if graph.op(node) == Opcode::Projection
                && matches!(graph.value_type(node), ValueType::Control)
            {
                projections
                    .entry((graph.input(node, 0), graph.payload(node)))
                    .or_default()
                    .push(node);
            }
        }
    }
    for (branch, taken) in folds {
        let predecessor = graph.resolve(graph.control(branch));
        for &projection in projections.get(&(branch, taken)).into_iter().flatten() {
            graph.replace(projection, predecessor);
        }
        // Untaken projections temporarily retain this dead source, making
        // them unreachable. The reachability/DCE sweep below retires them.
        graph.remove_node(branch);
        stats.branches_folded += 1;
    }
    graph.resolve_edges();
    let cfg = ControlFlow::new(graph);
    let regions: Vec<_> = graph
        .ids()
        .filter(|node| graph.op(*node) == Opcode::Region && cfg.contains(*node))
        .collect();
    for region in regions {
        let keep: Vec<_> = graph
            .inputs(region)
            .iter()
            .map(|jump| cfg.contains(*jump))
            .collect();
        stats.predecessor_edges_removed +=
            CfgEditor::new(graph).retain_predecessors(region, &keep)?;
    }
    graph.resolve_edges();
    stats.phis_folded = fold_redundant_phis(graph);
    graph.resolve_edges();

    let mut live = vec![false; graph.len()];
    let mut pending: Vec<NodeId> = graph
        .ids()
        .filter(|node| {
            (is_control_flow(graph, *node) && cfg.contains(*node))
                || (is_anchored_value(graph.op(*node)) && cfg.contains(graph.control(*node)))
        })
        .collect();
    while let Some(node) = pending.pop() {
        let node = graph.resolve(node);
        if node.is_none() || graph.is_dead(node) || live[node.index()] {
            continue;
        }
        live[node.index()] = true;
        pending.extend(graph.inputs(node).iter().copied());
        pending.push(graph.control(node));
    }
    let dead: Vec<_> = graph.ids().filter(|node| !live[node.index()]).collect();
    for node in dead {
        graph.remove_node(node);
    }
    stats.nodes_removed = before - graph.live_count();
    Ok(stats)
}

#[cfg(test)]
#[path = "simplify_unreachable_tests.rs"]
mod unreachable_tests;
