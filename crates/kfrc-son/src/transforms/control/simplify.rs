//! Fold literal branches, trim merge slots, then discard unreachable graphs.
//!
//! This pass does not speculate values or infer conditions from poison. Fixed
//! operations on retained control paths, including unused Div/Rem, are roots.

use kfrc_body::TypeShape;
use std::collections::HashMap;

use crate::analysis::cfg::{ControlFlow, is_anchored_value, is_control_flow};
use crate::transforms::utils::cfg::{CfgEditError, CfgEditor};
use crate::{FunctionGraph, NodeId, Opcode, ValueType};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ControlStats {
    pub branches_folded: usize,
    pub predecessor_edges_removed: usize,
    pub nodes_removed: usize,
}

pub fn run(graph: &mut FunctionGraph) -> Result<ControlStats, CfgEditError> {
    graph.resolve_edges();
    let before = graph.ids().count();
    let mut stats = ControlStats::default();
    // Branch folding only forwards/removes existing nodes, so one immutable
    // projection index remains valid through this loop. Avoid rescanning the
    // entire graph for every selected edge.
    let mut projections: HashMap<(NodeId, u32), Vec<NodeId>> = HashMap::new();
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
    stats.nodes_removed = before - graph.ids().count();
    Ok(stats)
}
