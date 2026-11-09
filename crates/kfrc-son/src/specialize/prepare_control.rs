//! Definition-only control cleanup. Full scalar optimization has already run;
//! CFG edits revisit only changed Phi values and their existing users. No type
//! answer provider is available here, so an unresolved question stays opaque.
use super::PreparationStats;
use crate::analysis::uses::{UseKind, Uses};
use crate::transforms::{control::simplify, scalar::seam};
use crate::{FunctionGraph, NodeId, Opcode, ValueType};
use std::collections::{HashSet, VecDeque};

pub(super) fn run(graph: &mut FunctionGraph, stats: &mut PreparationStats) -> Result<(), String> {
    loop {
        let uses = Uses::new(graph);
        let mut phis = Vec::new();
        for node in graph.ids() {
            stats.control_snapshot_nodes += 1;
            stats.control_snapshot_edges +=
                graph.inputs(node).len() + usize::from(!graph.control(node).is_none());
            if graph.op(node) == Opcode::Phi && graph.value_type(node) != &ValueType::Effect {
                phis.push((node, graph.inputs(node).to_vec()));
            }
        }
        let control =
            simplify::run(graph).map_err(|error| format!("R control preparation: {error:?}"))?;
        stats.control_simplification_runs += 1;
        stats.branches_folded += control.branches_folded;
        stats.predecessor_edges_removed += control.predecessor_edges_removed;
        stats.control_nodes_removed += control.nodes_removed;

        let mut changed = Vec::new();
        let mut seeds = HashSet::new();
        for (phi, before) in phis {
            let replacement = graph.resolve(phi);
            if graph.is_dead(replacement) {
                continue;
            }
            if replacement != phi || graph.inputs(phi) != before {
                changed.push(phi);
                seeds.insert(replacement);
                for usage in uses.of(phi) {
                    let user = graph.resolve(usage.user);
                    if !graph.is_dead(user) {
                        seeds.insert(user);
                    }
                }
            }
        }
        if seeds.is_empty() {
            break;
        }
        // Determine which branch conditions could change before the scalar
        // worklist forwards nodes. Reuse this snapshot; new scalar nodes never
        // introduce new branches. Unrelated interiors are not rule seeds.
        let branches = affected_branches(graph, &uses, &changed, stats);
        let mut seeds: Vec<_> = seeds.into_iter().collect();
        seeds.sort_unstable();
        let scalar = seam::run_seeded(graph, &seeds);
        stats.control_rule_nodes_indexed += scalar.nodes_indexed;
        stats.control_rule_nodes_visited += scalar.nodes_visited;
        stats.control_constants_folded += scalar.constants_folded;
        stats.control_identities_simplified += scalar.identities_simplified;
        stats.control_fields_folded += scalar.fields_folded;
        stats.control_variants_folded += scalar.variants_folded;
        graph.resolve_edges();
        if !branches
            .into_iter()
            .any(|branch| known_branch(graph, branch))
        {
            break;
        }
        // Each extra sweep is justified by a newly constant live branch and
        // removes that branch. The finite source graph bounds the exchanges;
        // no arbitrary round cap leaves independent control for every instance.
    }
    Ok(())
}

fn affected_branches(
    graph: &FunctionGraph,
    uses: &Uses,
    changed: &[NodeId],
    stats: &mut PreparationStats,
) -> Vec<NodeId> {
    let mut pending: VecDeque<_> = changed.iter().copied().collect();
    let mut visited = HashSet::new();
    let mut branches = Vec::new();
    while let Some(node) = pending.pop_front() {
        if !visited.insert(node) {
            continue;
        }
        for usage in uses.of(node) {
            stats.control_changed_use_edges += 1;
            if !matches!(usage.kind, UseKind::Operand(_)) {
                continue;
            }
            let user = usage.user;
            if graph.is_dead(user) {
                continue;
            }
            match graph.op(user) {
                Opcode::If | Opcode::VariantSwitch => branches.push(user),
                op if !op.is_fixed() => pending.push_back(user),
                _ => {}
            }
        }
    }
    branches
}

fn known_branch(graph: &FunctionGraph, branch: NodeId) -> bool {
    if graph.is_dead(branch) {
        return false;
    }
    let condition = graph.resolve(graph.input(branch, 0));
    match graph.op(branch) {
        Opcode::If => {
            graph.op(condition) == Opcode::Const && matches!(graph.constant_value(condition), 0 | 1)
        }
        Opcode::VariantSwitch => graph.op(condition) == Opcode::MakeVariant,
        _ => false,
    }
}
