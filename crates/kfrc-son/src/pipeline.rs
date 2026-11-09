//! Orchestration for the currently implemented graph optimization pipeline.
//!
//! Local rules combine integer folding, semantic aggregate folding and pure
//! value numbering. The checked specialized entry also prunes constant control
//! and validates every boundary. Definition-level R and the remaining planned
//! passes are not implemented here. This entry owns compaction and statistics.

use crate::analysis::memory::MemoryFacts;
use crate::graph::{FunctionGraph, GraphLevel, GraphState, NodeId, Opcode};
use crate::transforms::control::{simplify, thread_variants, variants};
use crate::transforms::memory::{forward, observation, partition, promotion};
use crate::transforms::scalar::{aggregate, division, gvn, instcombine, seam};
pub use crate::transforms::utils::cfg::CfgEditError;
use crate::verify::{VerifyError, verify_graph};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct OptimizeStats {
    pub constants_folded: usize,
    pub identities_simplified: usize,
    pub fields_folded: usize,
    pub variants_folded: usize,
    pub aggregates_simplified: usize,
    pub values_merged: usize,
    pub loads_forwarded: usize,
    pub observations_removed: usize,
    pub observation_nodes_indexed: usize,
    pub observation_node_visits: usize,
    pub rule_nodes_indexed: usize,
    pub rule_nodes_visited: usize,
    pub slots_promoted: usize,
    pub loads_removed: usize,
    pub stores_removed: usize,
    pub memory_node_visits: usize,
    pub memory_edge_visits: usize,
    pub local_accesses: usize,
    pub const_loads: usize,
    pub variant_regions_threaded: usize,
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
    pub predecessor_edges_removed: usize,
    pub nodes_removed: usize,
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

/// Checked semantic pipeline. CFG edits maintain Region/Phi/effect structure;
/// local rules run again after pruning to simplify newly exposed values.
pub fn optimize_specialized_graph(
    graph: &mut FunctionGraph,
) -> Result<OptimizeStats, PipelineError> {
    if graph.level != GraphLevel::Specialized {
        return Err(PipelineError::NotSpecialized);
    }
    verify_graph(graph).map_err(PipelineError::Verification)?;
    let mut stats = optimize_graph(graph);
    verify_graph(graph).map_err(PipelineError::Verification)?;
    promote(graph, None, &mut stats);
    remove_redundant_observations(graph, &mut stats);
    let threaded = thread_variants::run(graph).map_err(PipelineError::Control)?;
    stats.variant_regions_threaded += threaded.regions_threaded;
    stats.variant_values_cloned += threaded.values_cloned;
    let control = simplify::run(graph).map_err(PipelineError::Control)?;
    // Check the atomic CFG mutation before allowing another value transform.
    verify_graph(graph).map_err(PipelineError::Verification)?;
    let after_control = optimize_graph(graph);
    stats.constants_folded += after_control.constants_folded;
    stats.identities_simplified += after_control.identities_simplified;
    stats.fields_folded += after_control.fields_folded;
    stats.variants_folded += after_control.variants_folded;
    stats.aggregates_simplified += after_control.aggregates_simplified;
    stats.values_merged += after_control.values_merged;
    stats.loads_forwarded += after_control.loads_forwarded;
    stats.divisions_reduced += after_control.divisions_reduced;
    stats.remainders_reduced += after_control.remainders_reduced;
    stats.division_node_visits += after_control.division_node_visits;
    stats.division_proof_visits += after_control.division_proof_visits;
    stats.branches_folded = control.branches_folded;
    stats.predecessor_edges_removed = control.predecessor_edges_removed;
    stats.nodes_removed = control.nodes_removed;
    stats.nodes_after = graph.len();
    partition_memory(graph, &mut stats);
    verify_graph(graph).map_err(PipelineError::Verification)?;
    Ok(stats)
}

/// Simplify only splice seams and replacement users after inlining. CFG edits
/// can expose new Phi operands, which become the next seeds. Node identities
/// stay stable until final compaction; callers must then discard their seeds.
pub fn optimize_specialized_seams(
    graph: &mut FunctionGraph,
    seeds: &[NodeId],
    argument_slots: &[NodeId],
) -> Result<OptimizeStats, PipelineError> {
    if graph.level != GraphLevel::Specialized {
        return Err(PipelineError::NotSpecialized);
    }
    verify_graph(graph).map_err(PipelineError::Verification)?;
    graph.state = GraphState::Sealed;
    let mut stats = OptimizeStats {
        nodes_before: graph.len(),
        ..OptimizeStats::default()
    };
    let mut pending = seeds.to_vec();
    let candidates = argument_slots
        .iter()
        .chain(seeds)
        .copied()
        .map(|node| graph.resolve(node))
        .filter(|node| {
            !node.is_none() && !graph.is_dead(*node) && graph.op(*node) == Opcode::LocalSlot
        })
        .map(|node| graph.payload(node))
        .collect();
    pending.extend(promote(graph, Some(&candidates), &mut stats));
    // Pointer/aggregate identities can expose an already observed value.
    // Removing its redundant observation then exposes a Region's direct
    // VariantSwitch user, allowing predecessor threading before layout.
    optimize_seeds(graph, &pending, &mut stats);
    pending = remove_redundant_observations(graph, &mut stats);
    let threaded = thread_variants::run(graph).map_err(PipelineError::Control)?;
    stats.variant_regions_threaded += threaded.regions_threaded;
    stats.variant_values_cloned += threaded.values_cloned;
    pending.extend(threaded.dirty_nodes);
    // A bounded number of seam/CFG exchanges handles freshly constant Phi
    // conditions. No unchanged function interior is sent through scalar rules.
    for _ in 0..4 {
        optimize_seeds(graph, &pending, &mut stats);
        // Capture users before CFG pruning forwards a one-input Phi, so those
        // consumers remain seeds even after their input is no longer a Phi.
        let phis: std::collections::HashSet<_> = graph
            .ids()
            .filter(|node| graph.op(*node) == Opcode::Phi)
            .collect();
        let phi_users: Vec<_> = graph
            .ids()
            .filter(|node| {
                phis.contains(node) || graph.inputs(*node).iter().any(|input| phis.contains(input))
            })
            .collect();
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
    graph.compact();
    stats.nodes_after = graph.len();
    partition_memory(graph, &mut stats);
    verify_graph(graph).map_err(PipelineError::Verification)?;
    Ok(stats)
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
    stats.observation_nodes_indexed += observed.nodes_indexed;
    stats.observation_node_visits += observed.node_visits;
    observed.dirty_nodes
}

fn promote(
    graph: &mut FunctionGraph,
    candidates: Option<&std::collections::HashSet<u32>>,
    stats: &mut OptimizeStats,
) -> Vec<NodeId> {
    if candidates.is_some_and(|candidates| candidates.is_empty()) {
        return Vec::new();
    }
    let mut promoted = promotion::run(graph, candidates);
    stats.slots_promoted += promoted.slots_promoted;
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
