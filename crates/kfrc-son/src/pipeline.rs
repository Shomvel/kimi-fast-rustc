//! Orchestration for the currently implemented graph optimization pipeline.
//!
//! Local rules combine integer folding, semantic aggregate folding and pure
//! value numbering. The checked specialized entry also prunes constant control
//! and validates every boundary. Definition-level R and the remaining planned
//! passes are not implemented here. This entry owns compaction and statistics.

use crate::analysis::memory::MemoryFacts;
use crate::graph::{FunctionGraph, GraphLevel, GraphState, NodeId, Opcode};
use crate::transforms::control::{
    conditions, loop_simplify, noreturn, sccp, simplify, thread_variants, variants,
};
use crate::transforms::loops;
use crate::transforms::memory::{dse, forward, observation, partition, promotion};
use crate::transforms::scalar::{aggregate, division, gvn, instcombine, pre, ranges, seam};
pub use crate::transforms::utils::cfg::CfgEditError;
use crate::verify::{VerifyError, verify_pipeline_step};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct OptimizeStats {
    pub constants_folded: usize,
    pub identities_simplified: usize,
    pub fields_folded: usize,
    pub variants_folded: usize,
    pub aggregates_simplified: usize,
    pub values_merged: usize,
    pub loads_forwarded: usize,
    pub frozen_loads_reused: usize,
    pub calls_crossed_by_forwarding: usize,
    pub call_permissions_used: usize,
    pub effect_phis_crossed: usize,
    pub disjoint_writes_crossed: usize,
    pub location_node_visits: usize,
    pub location_budget_bailouts: usize,
    pub forward_walk_steps: usize,
    pub forward_budget_bailouts: usize,
    pub observations_removed: usize,
    pub observed_conditions_folded: usize,
    pub observation_nodes_indexed: usize,
    pub observation_node_visits: usize,
    pub rule_nodes_indexed: usize,
    pub rule_nodes_visited: usize,
    pub slots_promoted: usize,
    pub partitions_promoted: usize,
    pub promotion_partition_visits: usize,
    pub promotion_partition_budget_bailouts: usize,
    pub loads_removed: usize,
    pub stores_removed: usize,
    pub dead_stores_removed: usize,
    pub dse_node_visits: usize,
    pub dse_walk_steps: usize,
    pub dse_budget_bailouts: usize,
    pub memory_node_visits: usize,
    pub memory_edge_visits: usize,
    pub local_accesses: usize,
    pub const_loads: usize,
    pub variant_regions_threaded: usize,
    pub variant_control_spines_collapsed: usize,
    pub variant_values_cloned: usize,
    pub branches_normalized: usize,
    pub witnesses_added: usize,
    pub normalization_nodes_indexed: usize,
    pub normalization_use_edges_visited: usize,
    pub divisions_reduced: usize,
    pub remainders_reduced: usize,
    pub division_node_visits: usize,
    pub division_proof_visits: usize,
    pub branches_folded: usize,
    pub conditions_folded: usize,
    pub sccp_constants: usize,
    pub sccp_visits: usize,
    pub sccp_budget_bailouts: usize,
    pub fact_queries: usize,
    pub fact_node_visits: usize,
    pub condition_receipts_checked: usize,
    pub range_comparisons_folded: usize,
    pub range_fact_queries: usize,
    pub range_node_visits: usize,
    pub predecessor_edges_removed: usize,
    pub nodes_removed: usize,
    pub loops_unrolled: usize,
    pub loop_iterations_cloned: usize,
    pub loop_nodes_cloned: usize,
    pub loop_analysis_visits: usize,
    pub loop_budget_bailouts: usize,
    pub loop_preheaders_created: usize,
    pub loop_entry_phis_created: usize,
    pub loop_normalization_nodes_added: usize,
    pub loop_normalization_visits: usize,
    pub loop_normalization_budget_bailouts: usize,
    pub loop_products_reduced: usize,
    pub loop_recurrences_created: usize,
    pub loop_recurrence_nodes_added: usize,
    pub loop_recurrence_analysis_visits: usize,
    pub loop_recurrence_budget_bailouts: usize,
    pub pre_expressions_replaced: usize,
    pub pre_expressions_inserted: usize,
    pub pre_phis_created: usize,
    pub pre_analysis_visits: usize,
    pub pre_budget_bailouts: usize,
    pub nodes_before: usize,
    pub nodes_after: usize,
}

pub fn optimize_graph(graph: &mut FunctionGraph) -> OptimizeStats {
    let nodes_before = graph.len();
    // A previous successful verification no longer covers a graph mutation.
    graph.state = GraphState::Sealed;
    let semantic = matches!(graph.level, GraphLevel::Rust | GraphLevel::Specialized);
    let mut aggregate_stats = if semantic {
        aggregate::run(graph)
    } else {
        aggregate::AggregateStats::default()
    };
    let stats = instcombine::run(graph);
    let division_stats = division::run(graph);
    let gvn_stats = if semantic {
        gvn::run(graph)
    } else {
        gvn::GvnStats::default()
    };
    let memory_stats = if semantic {
        forward::run(graph)
    } else {
        forward::ForwardStats::default()
    };
    let store_stats = dse::run(graph);
    if semantic {
        let after_gvn = aggregate::run(graph);
        aggregate_stats.fields_folded += after_gvn.fields_folded;
        aggregate_stats.variants_folded += after_gvn.variants_folded;
        aggregate_stats.aggregates_simplified += after_gvn.aggregates_simplified;
    }
    // Compaction renumbers NodeIds and belongs at the pipeline boundary.
    graph.compact();
    OptimizeStats {
        constants_folded: stats.constants_folded,
        identities_simplified: stats.identities_simplified,
        fields_folded: aggregate_stats.fields_folded,
        variants_folded: aggregate_stats.variants_folded,
        aggregates_simplified: aggregate_stats.aggregates_simplified,
        values_merged: gvn_stats.values_merged,
        loads_forwarded: memory_stats.loads_forwarded,
        frozen_loads_reused: memory_stats.frozen_loads_reused,
        calls_crossed_by_forwarding: memory_stats.calls_crossed_by_forwarding,
        call_permissions_used: memory_stats.call_permissions_used,
        effect_phis_crossed: memory_stats.effect_phis_crossed,
        disjoint_writes_crossed: memory_stats.disjoint_writes_crossed,
        location_node_visits: memory_stats.location_node_visits,
        location_budget_bailouts: memory_stats.location_budget_bailouts,
        forward_walk_steps: memory_stats.walk_steps,
        forward_budget_bailouts: memory_stats.budget_bailouts,
        stores_removed: store_stats.stores_removed,
        dead_stores_removed: store_stats.stores_removed,
        dse_node_visits: store_stats.node_visits,
        dse_walk_steps: store_stats.walk_steps,
        dse_budget_bailouts: store_stats.budget_bailouts,
        divisions_reduced: division_stats.divisions_reduced,
        remainders_reduced: division_stats.remainders_reduced,
        division_node_visits: division_stats.nodes_visited,
        division_proof_visits: division_stats.proof_nodes_visited,
        nodes_before,
        nodes_after: graph.len(),
        ..OptimizeStats::default()
    }
}

#[derive(Debug)]
pub enum PipelineError {
    NotSpecialized,
    Verification(VerifyError),
    Control(CfgEditError),
}

impl std::fmt::Display for PipelineError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotSpecialized => {
                f.write_str("semantic optimization requires a Specialized graph")
            }
            Self::Verification(error) => write!(f, "graph verification: {error}"),
            Self::Control(error) => write!(f, "control simplification: {error}"),
        }
    }
}

impl std::error::Error for PipelineError {}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CallEffectsStats {
    pub loads_forwarded: usize,
    pub calls_crossed: usize,
    pub noreturn_calls: usize,
}

/// Consume newly inferred call effects. No-return calls retain their effects
/// and discard normal continuations; no-write proofs permit load forwarding.
pub fn optimize_call_effects(graph: &mut FunctionGraph) -> Result<CallEffectsStats, PipelineError> {
    if graph.level != GraphLevel::Specialized {
        return Err(PipelineError::NotSpecialized);
    }
    verify_pipeline_step(graph).map_err(PipelineError::Verification)?;
    let noreturn_calls = noreturn::run(graph);
    if noreturn_calls != 0 {
        simplify::run(graph).map_err(PipelineError::Control)?;
    }
    let forwarded = forward::run(graph);
    if forwarded.loads_forwarded != 0 || noreturn_calls != 0 {
        graph.compact();
        verify_pipeline_step(graph).map_err(PipelineError::Verification)?;
    }
    Ok(CallEffectsStats {
        loads_forwarded: forwarded.loads_forwarded,
        calls_crossed: forwarded.calls_crossed_by_forwarding,
        noreturn_calls,
    })
}

/// Checked semantic pipeline. CFG edits maintain Region/Phi/effect structure;
/// local rules run again after pruning to simplify newly exposed values.
pub fn optimize_specialized_graph(
    graph: &mut FunctionGraph,
) -> Result<OptimizeStats, PipelineError> {
    if graph.level != GraphLevel::Specialized {
        return Err(PipelineError::NotSpecialized);
    }
    verify_pipeline_step(graph).map_err(PipelineError::Verification)?;
    let mut stats = optimize_graph(graph);
    fold_identity_transmutes(graph);
    verify_pipeline_step(graph).map_err(PipelineError::Verification)?;
    promote(graph, None, &mut stats);
    remove_redundant_observations(graph, &mut stats);
    let threaded = thread_variants::run(graph).map_err(PipelineError::Control)?;
    stats.variant_regions_threaded += threaded.regions_threaded;
    stats.variant_control_spines_collapsed += threaded.control_spines_collapsed;
    stats.variant_values_cloned += threaded.values_cloned;
    let range_dirty = fold_range_conditions(graph, &mut stats);
    optimize_seeds(graph, &range_dirty, &mut stats);
    fold_scoped_conditions(graph, &mut stats)?;
    propagate_constants(graph, &mut stats);
    let control = simplify::run(graph).map_err(PipelineError::Control)?;
    // Check the atomic CFG mutation before allowing another value transform.
    verify_pipeline_step(graph).map_err(PipelineError::Verification)?;
    let after_control = optimize_graph(graph);
    stats.constants_folded += after_control.constants_folded;
    stats.identities_simplified += after_control.identities_simplified;
    stats.fields_folded += after_control.fields_folded;
    stats.variants_folded += after_control.variants_folded;
    stats.aggregates_simplified += after_control.aggregates_simplified;
    stats.values_merged += after_control.values_merged;
    stats.loads_forwarded += after_control.loads_forwarded;
    stats.frozen_loads_reused += after_control.frozen_loads_reused;
    stats.calls_crossed_by_forwarding += after_control.calls_crossed_by_forwarding;
    stats.call_permissions_used += after_control.call_permissions_used;
    stats.effect_phis_crossed += after_control.effect_phis_crossed;
    stats.disjoint_writes_crossed += after_control.disjoint_writes_crossed;
    stats.location_node_visits += after_control.location_node_visits;
    stats.location_budget_bailouts += after_control.location_budget_bailouts;
    stats.forward_walk_steps += after_control.forward_walk_steps;
    stats.forward_budget_bailouts += after_control.forward_budget_bailouts;
    stats.stores_removed += after_control.stores_removed;
    stats.dead_stores_removed += after_control.dead_stores_removed;
    stats.dse_node_visits += after_control.dse_node_visits;
    stats.dse_walk_steps += after_control.dse_walk_steps;
    stats.dse_budget_bailouts += after_control.dse_budget_bailouts;
    stats.divisions_reduced += after_control.divisions_reduced;
    stats.remainders_reduced += after_control.remainders_reduced;
    stats.division_node_visits += after_control.division_node_visits;
    stats.division_proof_visits += after_control.division_proof_visits;
    stats.branches_folded = control.branches_folded;
    stats.predecessor_edges_removed = control.predecessor_edges_removed;
    stats.nodes_removed = control.nodes_removed;
    let unrolled = unroll_small_loops(graph, &mut stats)?;
    let strength_reduced = reduce_loop_products(graph, &mut stats)?;
    let redundancy_removed = eliminate_partial_redundancy(graph, &mut stats);
    if unrolled || strength_reduced || redundancy_removed {
        eliminate_exposed_values(graph, &mut stats)?;
        graph.compact();
    }
    stats.nodes_after = graph.len();
    partition_memory(graph, &mut stats);
    verify_pipeline_step(graph).map_err(PipelineError::Verification)?;
    Ok(stats)
}

/// Simplify splice seams and replacement users, then eliminate redundancy
/// exposed by the substitutions. CFG edits can expose new Phi operands, which
/// become the next seeds. Node identities stay stable until final compaction;
/// callers must then discard their seeds.
pub fn optimize_specialized_seams(
    graph: &mut FunctionGraph,
    seeds: &[NodeId],
    argument_slots: &[NodeId],
) -> Result<OptimizeStats, PipelineError> {
    if graph.level != GraphLevel::Specialized {
        return Err(PipelineError::NotSpecialized);
    }
    verify_pipeline_step(graph).map_err(PipelineError::Verification)?;
    graph.state = GraphState::Sealed;
    let mut stats = OptimizeStats {
        nodes_before: graph.len(),
        ..OptimizeStats::default()
    };
    let mut pending = seeds.to_vec();
    pending.extend(fold_identity_transmutes(graph));
    // Scalar substitutions can make an array index fixed. Follow its changed
    // address user to the owning slot before collecting promotion candidates.
    optimize_seeds(graph, &pending, &mut stats);
    let slot_seeds: Vec<_> = argument_slots.iter().chain(&pending).copied().collect();
    let candidates = promotion::candidate_slots(graph, &slot_seeds);
    let promoted = promote(graph, Some(&candidates), &mut stats);
    // Pointer/aggregate identities can expose an already observed value.
    // Removing its redundant observation then exposes a Region's direct
    // VariantSwitch user, allowing predecessor threading before layout.
    optimize_seeds(graph, &promoted, &mut stats);
    pending = remove_redundant_observations(graph, &mut stats);
    let threaded = thread_variants::run(graph).map_err(PipelineError::Control)?;
    stats.variant_regions_threaded += threaded.regions_threaded;
    stats.variant_control_spines_collapsed += threaded.control_spines_collapsed;
    stats.variant_values_cloned += threaded.values_cloned;
    pending.extend(threaded.dirty_nodes);
    // A bounded number of seam/CFG exchanges handles freshly constant Phi
    // conditions. No unchanged function interior is sent through scalar rules.
    for _ in 0..4 {
        optimize_seeds(graph, &pending, &mut stats);
        let range_dirty = fold_range_conditions(graph, &mut stats);
        optimize_seeds(graph, &range_dirty, &mut stats);
        // Capture users before CFG pruning forwards a one-input Phi, so those
        // consumers remain seeds even after their input is no longer a Phi.
        let phis: kfrc_body::fx_hash::HashSet<_> = graph
            .ids()
            .filter(|node| graph.op(*node) == Opcode::Phi)
            .collect();
        let phi_users: Vec<_> = graph
            .ids()
            .filter(|node| {
                phis.contains(node) || graph.inputs(*node).iter().any(|input| phis.contains(input))
            })
            .collect();
        fold_scoped_conditions(graph, &mut stats)?;
        let control = simplify::run(graph).map_err(PipelineError::Control)?;
        stats.branches_folded += control.branches_folded;
        stats.predecessor_edges_removed += control.predecessor_edges_removed;
        stats.nodes_removed += control.nodes_removed;
        if control.predecessor_edges_removed == 0 {
            break;
        }
        // CFG pruning leaves NodeIds intact. Queue surviving Phi consumers;
        // the scalar worklist propagates only successful replacements.
        pending = phi_users;
    }
    // Seeds cover splice seams. A spliced body can still hold a constant
    // condition no seed reaches (a substituted runtime-check flag), so one
    // linear sweep folds it and removes the dead branch, as LLVM runs
    // InstCombine/SimplifyCFG after its inliner.
    let swept = instcombine::run(graph);
    stats.constants_folded += swept.constants_folded;
    stats.identities_simplified += swept.identities_simplified;
    if swept.constants_folded + swept.identities_simplified > 0 {
        let control = simplify::run(graph).map_err(PipelineError::Control)?;
        stats.branches_folded += control.branches_folded;
        stats.predecessor_edges_removed += control.predecessor_edges_removed;
        stats.nodes_removed += control.nodes_removed;
    }
    let propagated = propagate_constants(graph, &mut stats);
    if !propagated.is_empty() {
        let control = simplify::run(graph).map_err(PipelineError::Control)?;
        stats.branches_folded += control.branches_folded;
        stats.predecessor_edges_removed += control.predecessor_edges_removed;
        stats.nodes_removed += control.nodes_removed;
        optimize_seeds(graph, &propagated, &mut stats);
    }
    // The immutable source was simplified before substitution, but two source
    // expressions/addresses can become equal only at this boundary. Revisit
    // numbering and effects once per batch, never once per spliced call. An
    // unchanged instance still uses the cached definition's results.
    let unrolled = unroll_small_loops(graph, &mut stats)?;
    let strength_reduced = reduce_loop_products(graph, &mut stats)?;
    let redundancy_removed = eliminate_partial_redundancy(graph, &mut stats);
    if unrolled
        || strength_reduced
        || redundancy_removed
        || !seeds.is_empty()
        || !argument_slots.is_empty()
        || stats.constants_folded
            + stats.identities_simplified
            + stats.fields_folded
            + stats.branches_folded
            + stats.slots_promoted
            > 0
    {
        eliminate_exposed_values(graph, &mut stats)?;
    }
    graph.compact();
    stats.nodes_after = graph.len();
    partition_memory(graph, &mut stats);
    verify_pipeline_step(graph).map_err(PipelineError::Verification)?;
    Ok(stats)
}

fn reduce_loop_products(
    graph: &mut FunctionGraph,
    stats: &mut OptimizeStats,
) -> Result<bool, PipelineError> {
    let reduced = loops::strength_reduce::run(graph);
    stats.loop_products_reduced += reduced.products_reduced;
    stats.loop_recurrences_created += reduced.recurrences_created;
    stats.loop_recurrence_nodes_added += reduced.nodes_added;
    stats.loop_recurrence_analysis_visits += reduced.analysis_visits;
    stats.loop_recurrence_budget_bailouts += reduced.budget_bailouts;
    if reduced.products_reduced == 0 {
        return Ok(false);
    }
    verify_pipeline_step(graph).map_err(PipelineError::Verification)?;
    optimize_seeds(graph, &reduced.dirty_nodes, stats);
    Ok(true)
}

fn eliminate_partial_redundancy(graph: &mut FunctionGraph, stats: &mut OptimizeStats) -> bool {
    let reduced = pre::run(graph);
    stats.pre_expressions_replaced += reduced.expressions_replaced;
    stats.pre_expressions_inserted += reduced.expressions_inserted;
    stats.pre_phis_created += reduced.phis_created;
    stats.pre_analysis_visits += reduced.analysis_visits;
    stats.pre_budget_bailouts += reduced.budget_bailouts;
    if reduced.expressions_replaced == 0 {
        return false;
    }
    optimize_seeds(graph, &reduced.dirty_nodes, stats);
    true
}

fn unroll_small_loops(
    graph: &mut FunctionGraph,
    stats: &mut OptimizeStats,
) -> Result<bool, PipelineError> {
    let normalized = loop_simplify::run(graph).map_err(PipelineError::Control)?;
    stats.loop_preheaders_created += normalized.preheaders_created;
    stats.loop_entry_phis_created += normalized.phis_created;
    stats.loop_normalization_nodes_added += normalized.nodes_added;
    stats.loop_normalization_visits += normalized.analysis_visits;
    stats.loop_normalization_budget_bailouts += normalized.budget_bailouts;
    if normalized.preheaders_created != 0 {
        verify_pipeline_step(graph).map_err(PipelineError::Verification)?;
    }
    let unrolled = loops::run(graph).map_err(PipelineError::Control)?;
    stats.loops_unrolled += unrolled.loops_unrolled;
    stats.loop_iterations_cloned += unrolled.iterations_cloned;
    stats.loop_nodes_cloned += unrolled.nodes_cloned;
    stats.loop_analysis_visits += unrolled.analysis_visits;
    stats.loop_budget_bailouts += unrolled.budget_bailouts;
    if unrolled.loops_unrolled == 0 {
        return Ok(normalized.preheaders_created != 0);
    }
    verify_pipeline_step(graph).map_err(PipelineError::Verification)?;
    let mut dirty = unrolled.dirty_nodes;
    optimize_seeds(graph, &dirty, stats);
    let slot_seeds: Vec<_> = unrolled
        .exposed_slots
        .iter()
        .chain(&dirty)
        .copied()
        .collect();
    let candidates = promotion::candidate_slots(graph, &slot_seeds);
    dirty.extend(promote(graph, Some(&candidates), stats));
    optimize_seeds(graph, &dirty, stats);
    let range_dirty = fold_range_conditions(graph, stats);
    optimize_seeds(graph, &range_dirty, stats);
    fold_scoped_conditions(graph, stats)?;
    let propagated = propagate_constants(graph, stats);
    optimize_seeds(graph, &propagated, stats);
    let control = simplify::run(graph).map_err(PipelineError::Control)?;
    stats.branches_folded += control.branches_folded;
    stats.predecessor_edges_removed += control.predecessor_edges_removed;
    stats.nodes_removed += control.nodes_removed;
    Ok(true)
}

fn eliminate_exposed_values(
    graph: &mut FunctionGraph,
    stats: &mut OptimizeStats,
) -> Result<(), PipelineError> {
    let divisions = division::run(graph);
    let values = gvn::run(graph);
    let memory = forward::run(graph);
    let stores = dse::run(graph);
    stats.divisions_reduced += divisions.divisions_reduced;
    stats.remainders_reduced += divisions.remainders_reduced;
    stats.division_node_visits += divisions.nodes_visited;
    stats.division_proof_visits += divisions.proof_nodes_visited;
    stats.values_merged += values.values_merged;
    stats.loads_forwarded += memory.loads_forwarded;
    stats.frozen_loads_reused += memory.frozen_loads_reused;
    stats.calls_crossed_by_forwarding += memory.calls_crossed_by_forwarding;
    stats.call_permissions_used += memory.call_permissions_used;
    stats.effect_phis_crossed += memory.effect_phis_crossed;
    stats.disjoint_writes_crossed += memory.disjoint_writes_crossed;
    stats.location_node_visits += memory.location_node_visits;
    stats.location_budget_bailouts += memory.location_budget_bailouts;
    stats.forward_walk_steps += memory.walk_steps;
    stats.forward_budget_bailouts += memory.budget_bailouts;
    stats.stores_removed += stores.stores_removed;
    stats.dead_stores_removed += stores.stores_removed;
    stats.dse_node_visits += stores.node_visits;
    stats.dse_walk_steps += stores.walk_steps;
    stats.dse_budget_bailouts += stores.budget_bailouts;
    // Unrolling and PRE replace cyclic/merged values with ordinary expressions.
    // Revisit definedness at this batch boundary before deciding there is no
    // cleanup work; the observations ran against the original loop Phis.
    let observations_before = stats.observations_removed;
    let conditions_before = stats.observed_conditions_folded;
    let observation_dirty = remove_redundant_observations(graph, stats);
    optimize_seeds(graph, &observation_dirty, stats);
    let observations_removed = stats.observations_removed - observations_before;
    let conditions_folded = stats.observed_conditions_folded - conditions_before;
    if divisions.divisions_reduced
        + divisions.remainders_reduced
        + values.values_merged
        + memory.loads_forwarded
        + stores.stores_removed
        + observations_removed
        + conditions_folded
        == 0
    {
        return Ok(());
    }
    let aggregates = aggregate::run(graph);
    stats.fields_folded += aggregates.fields_folded;
    stats.variants_folded += aggregates.variants_folded;
    stats.aggregates_simplified += aggregates.aggregates_simplified;
    let scalar = instcombine::run(graph);
    stats.constants_folded += scalar.constants_folded;
    stats.identities_simplified += scalar.identities_simplified;
    let propagated = propagate_constants(graph, stats);
    optimize_seeds(graph, &propagated, stats);
    let control = simplify::run(graph).map_err(PipelineError::Control)?;
    stats.branches_folded += control.branches_folded;
    stats.predecessor_edges_removed += control.predecessor_edges_removed;
    stats.nodes_removed += control.nodes_removed;
    Ok(())
}

fn fold_range_conditions(graph: &mut FunctionGraph, stats: &mut OptimizeStats) -> Vec<NodeId> {
    graph.resolve_edges();
    let folded = ranges::run(graph);
    stats.range_comparisons_folded += folded.comparisons_folded;
    stats.range_fact_queries += folded.fact_queries;
    stats.range_node_visits += folded.node_visits;
    folded.dirty_nodes
}

fn fold_scoped_conditions(
    graph: &mut FunctionGraph,
    stats: &mut OptimizeStats,
) -> Result<(), PipelineError> {
    let folded = conditions::run(graph).map_err(PipelineError::Verification)?;
    stats.conditions_folded += folded.conditions_folded;
    stats.fact_queries += folded.fact_queries;
    stats.fact_node_visits += folded.fact_node_visits;
    stats.condition_receipts_checked += folded.receipts_checked;
    Ok(())
}

fn optimize_seeds(graph: &mut FunctionGraph, pending: &[NodeId], stats: &mut OptimizeStats) {
    if pending.is_empty() {
        return;
    }
    let local = seam::run_seeded(graph, pending);
    stats.constants_folded += local.constants_folded;
    stats.identities_simplified += local.identities_simplified;
    stats.fields_folded += local.fields_folded;
    stats.variants_folded += local.variants_folded;
    stats.aggregates_simplified += local.aggregates_simplified;
    stats.rule_nodes_indexed += local.nodes_indexed;
    stats.rule_nodes_visited += local.nodes_visited;
}

fn remove_redundant_observations(
    graph: &mut FunctionGraph,
    stats: &mut OptimizeStats,
) -> Vec<NodeId> {
    let observed = observation::run(graph);
    stats.observations_removed += observed.observations_removed;
    stats.observed_conditions_folded += observed.conditions_folded;
    stats.observation_nodes_indexed += observed.nodes_indexed;
    stats.observation_node_visits += observed.node_visits;
    observed.dirty_nodes
}

fn promote(
    graph: &mut FunctionGraph,
    candidates: Option<&kfrc_body::fx_hash::HashSet<u32>>,
    stats: &mut OptimizeStats,
) -> Vec<NodeId> {
    if candidates.is_some_and(|candidates| candidates.is_empty()) {
        return Vec::new();
    }
    let mut promoted = promotion::run(graph, candidates);
    stats.slots_promoted += promoted.slots_promoted;
    stats.partitions_promoted += promoted.partitions_promoted;
    stats.promotion_partition_visits += promoted.partition_node_visits;
    stats.promotion_partition_budget_bailouts += promoted.partition_budget_bailouts;
    stats.loads_removed += promoted.loads_removed;
    stats.stores_removed += promoted.stores_removed;
    stats.memory_node_visits += promoted.node_visits;
    stats.memory_edge_visits += promoted.edge_visits;
    if promoted.slots_promoted > 0 {
        // A newly promoted enum can expose Discriminant/field users that the
        // source builder could not associate through an address. Normalize
        // only after such a change; unchanged functions need no extra scan.
        let normalized = variants::normalize(graph);
        stats.branches_normalized += normalized.branches_normalized;
        stats.witnesses_added += normalized.witnesses_added;
        stats.normalization_nodes_indexed += normalized.nodes_indexed;
        stats.normalization_use_edges_visited += normalized.use_edges_visited;
        promoted.dirty_nodes.extend(normalized.dirty_nodes);
    }
    promoted.dirty_nodes
}

fn partition_memory(graph: &mut FunctionGraph, stats: &mut OptimizeStats) {
    let facts = MemoryFacts::new(graph);
    let partitions = partition::run(graph, &facts);
    stats.local_accesses += partitions.local_accesses;
    stats.const_loads += partitions.const_loads;
    stats.memory_node_visits += partitions.node_visits;
    stats.memory_edge_visits += partitions.edge_visits;
}

fn propagate_constants(graph: &mut FunctionGraph, stats: &mut OptimizeStats) -> Vec<NodeId> {
    let propagated = sccp::run(graph);
    stats.sccp_constants += propagated.constants;
    stats.sccp_visits += propagated.visits;
    stats.sccp_budget_bailouts += propagated.budget_bailouts;
    propagated.dirty
}

/// Same-type transmute observes its input but cannot change its representation.
/// Retaining RequireDefined preserves the S poison contract while releasing
/// pointer users from an opaque conversion, which can expose private storage.
fn fold_identity_transmutes(graph: &mut FunctionGraph) -> Vec<NodeId> {
    let uses = crate::analysis::uses::Uses::new(graph);
    let nodes: Vec<_> = graph
        .ids()
        .filter(|&n| graph.op(n) == Opcode::TransmuteValue)
        .collect();
    let mut dirty = Vec::new();
    for node in nodes {
        let value = graph.input(node, 1);
        let crate::ValueType::Tuple(fields) = graph.value_type(node) else {
            continue;
        };
        if fields.get(2) != Some(graph.value_type(value)) {
            continue;
        }
        for usage in uses.of(node) {
            if graph.op(usage.user) == Opcode::Projection && graph.payload(usage.user) == 2 {
                dirty.extend(uses.of(usage.user).iter().map(|u| u.user));
                graph.replace(usage.user, value);
            }
        }
        graph.set_opcode(node, Opcode::RequireDefined);
        graph.set_value_type(
            node,
            crate::ValueType::Tuple(vec![crate::ValueType::Control, crate::ValueType::Effect]),
        );
    }
    graph.resolve_edges();
    dirty
}
