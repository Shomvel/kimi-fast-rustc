//! Constant propagation for disposable candidates. No compaction or cloning
//! transforms run here, so the inlining queue keeps its NodeIds and depths.
use crate::transforms::{
    control::{sccp, simplify},
    memory::observation,
    scalar::seam,
};
use crate::verify::verify_pipeline_step;
use crate::{FunctionGraph, PipelineError};

const MAX_ROUNDS: usize = 4;
const MAX_STORED_NODES: usize = 4096;

#[derive(Default)]
pub(super) struct Cleanup {
    pub complete: bool,
    pub rounds: usize,
    pub nodes_indexed: usize,
    pub node_visits: usize,
    pub nodes_added: usize,
    pub branches_folded: usize,
    pub observations_removed: usize,
    pub conditions_folded: usize,
}

pub(super) fn run(
    graph: &mut FunctionGraph,
    mut reapply: impl FnMut(&mut FunctionGraph) -> usize,
) -> Result<Cleanup, PipelineError> {
    let mut stats = Cleanup::default();
    for _ in 0..MAX_ROUNDS {
        if graph.len() > MAX_STORED_NODES {
            return Ok(stats);
        }
        graph.resolve_edges();
        let before = graph.len();
        // Splicing can expose parameter fields that did not exist when the
        // frame was first refined. Keep fixed observations and queue NodeIds.
        let facts_applied = reapply(graph);
        let nodes: Vec<_> = graph.ids().collect();
        stats.nodes_indexed += nodes.len();
        let scalar = seam::run_seeded(graph, &nodes);
        stats.nodes_indexed += scalar.nodes_indexed;
        stats.node_visits += scalar.nodes_visited;
        graph.resolve_edges();
        let propagated = sccp::run(graph);
        stats.node_visits += propagated.visits;
        let observed = observation::run(graph);
        stats.nodes_indexed += observed.nodes_indexed;
        stats.node_visits += observed.node_visits;
        // simplify exposes its mutations rather than internal indexing counts.
        // Account this whole inventory explicitly in preparation telemetry.
        stats.nodes_indexed += graph.live_count();
        let control = simplify::run(graph).map_err(PipelineError::Control)?;
        stats.nodes_added += graph.len() - before;
        stats.branches_folded += control.branches_folded;
        stats.observations_removed += observed.observations_removed;
        stats.conditions_folded += observed.conditions_folded;
        stats.rounds += 1;
        verify_pipeline_step(graph).map_err(PipelineError::Verification)?;
        let changed = facts_applied
            + scalar.constants_folded
            + scalar.identities_simplified
            + scalar.fields_folded
            + scalar.variants_folded
            + scalar.aggregates_simplified
            + propagated.constants
            + observed.observations_removed
            + observed.conditions_folded
            + control.nodes_removed
            + control.predecessor_edges_removed
            + control.branches_folded;
        if changed == 0 {
            break;
        }
    }
    stats.complete = graph.len() <= MAX_STORED_NODES;
    Ok(stats)
}
