//! Fold literal branches, trim merge slots, then discard unreachable graphs.
//!
//! This pass does not speculate values or infer conditions from poison. Fixed
//! operations on retained control paths, including unused Div/Rem, are roots.

use kfrc_body::TypeShape;
use kfrc_body::fx_hash::HashMap;

use crate::analysis::cfg::{ControlFlow, is_anchored_value, is_control_flow};
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
        let mut same = None;
        let mut distinct = false;
        for &input in &graph.inputs(phi)[1..] {
            let input = graph.resolve(input);
            if input == phi {
                continue;
            }
            match same {
                None => same = Some(input),
                Some(value) if value == input => (),
                Some(_) => {
                    distinct = true;
                    break;
                }
            }
        }
        let Some(value) = same.filter(|_| !distinct) else {
            continue;
        };
        if value.is_none() || graph.value_type(value) != graph.value_type(phi) {
            continue;
        }
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

pub fn run(graph: &mut FunctionGraph) -> Result<ControlStats, CfgEditError> {
    run_impl(graph, true)
}

/// Reachability/Region/Phi repair without creating any new graph records.
/// Inliner growth accounts can be exhausted when this repair is mandatory.
pub(crate) fn prune(graph: &mut FunctionGraph) -> Result<ControlStats, CfgEditError> {
    run_impl(graph, false)
}

fn run_impl(graph: &mut FunctionGraph, fold_diamonds: bool) -> Result<ControlStats, CfgEditError> {
    graph.resolve_edges();
    let before = graph.live_count();
    let mut stats = ControlStats::default();
    if fold_diamonds {
        stats.branches_folded += super::diamonds::run(graph);
    }
    // Branch folding only forwards/removes existing nodes, so one immutable
    // projection index remains valid through this loop. Avoid rescanning the
    // entire graph for every selected edge.
    let mut projections: HashMap<(NodeId, u32), Vec<NodeId>> = HashMap::default();
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
    let branches: Vec<_> = graph
        .ids()
        .filter(|node| matches!(graph.op(*node), Opcode::If | Opcode::VariantSwitch))
        .collect();
    for branch in branches {
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
