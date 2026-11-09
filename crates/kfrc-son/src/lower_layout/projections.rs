//! Projection lookup during one layout-lowering transaction.
//!
//! Keys use raw input IDs, exactly like `FunctionGraph::find_projection`.
//! Register every new or retargeted projection. Deletion, forwarding, and type
//! changes need no notification: lookup checks the current node, never resolves
//! a replaced projection to an unrelated value. Stop querying before edge
//! resolution or compaction; only counters remain useful after finalization.

use super::LayoutStats;
use crate::{FunctionGraph, NodeId, Opcode};
use std::collections::{HashMap, VecDeque};

#[derive(Default)]
pub(super) struct ProjectionIndex {
    entries: HashMap<(NodeId, u32), VecDeque<NodeId>>,
    indexed: usize,
    queries: usize,
    misses: usize,
    candidates: usize,
}

impl ProjectionIndex {
    pub(super) fn new(graph: &FunctionGraph) -> Self {
        let mut index = Self::default();
        for node in graph.ids() {
            index.indexed += 1;
            index.register(graph, node);
        }
        index
    }

    pub(super) fn register(&mut self, graph: &FunctionGraph, node: NodeId) {
        if graph.is_dead(node) || graph.op(node) != Opcode::Projection {
            return;
        }
        let Some(&source) = graph.inputs(node).first() else {
            return;
        };
        let entries = self
            .entries
            .entry((source, graph.payload(node)))
            .or_default();
        // Seed and newly created nodes arrive in increasing order. Insertion
        // also supports re-registering an older projection after retargeting.
        if let Err(position) = entries.binary_search(&node) {
            entries.insert(position, node);
        }
    }

    pub(super) fn find(
        &mut self,
        graph: &FunctionGraph,
        source: NodeId,
        index: u32,
    ) -> Option<NodeId> {
        self.queries += 1;
        if let Some(entries) = self.entries.get_mut(&(source, index)) {
            while let Some(&node) = entries.front() {
                self.candidates += 1;
                if !graph.is_dead(node)
                    && graph.op(node) == Opcode::Projection
                    && graph.payload(node) == index
                    && graph.inputs(node).first() == Some(&source)
                {
                    return Some(node);
                }
                entries.pop_front();
            }
        }
        self.misses += 1;
        None
    }

    pub(super) fn write_stats(&self, stats: &mut LayoutStats) {
        stats.projection_indexed = self.indexed;
        stats.projection_queries = self.queries;
        stats.projection_cache_misses = self.misses;
        stats.projection_candidates_checked = self.candidates;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ValueType;
    use kfrc_body::{FunctionAbi, PassMode, RecordedTypes, TypeShape};

    fn graph() -> FunctionGraph {
        FunctionGraph::new(
            "projection_index",
            FunctionAbi {
                args: vec![],
                ret: PassMode::Ignore,
                c_variadic: false,
            },
            RecordedTypes::new(),
        )
    }

    fn source(graph: &mut FunctionGraph) -> NodeId {
        graph.add_node(
            Opcode::Start,
            NodeId::NONE,
            &[],
            0,
            ValueType::Tuple(vec![ValueType::Control, ValueType::Effect]),
        )
    }

    fn projection(graph: &mut FunctionGraph, source: NodeId, index: u32) -> NodeId {
        graph.add_node(
            Opcode::Projection,
            NodeId::NONE,
            &[source],
            index,
            ValueType::Control,
        )
    }

    #[test]
    fn mutations_preserve_the_scan_contract_without_resolving_replacements() {
        let mut graph = graph();
        let a = source(&mut graph);
        let b = source(&mut graph);
        let first = projection(&mut graph, a, 0);
        let duplicate = projection(&mut graph, a, 0);
        let mut index = ProjectionIndex::new(&graph);
        assert_eq!(index.find(&graph, a, 0), graph.find_projection(a, 0));

        // ABI lowering changes projection types without changing identity.
        let unit = graph.oracle.intern(TypeShape::Unit, None);
        graph.set_value_type(first, ValueType::LayoutValue(unit));
        assert_eq!(index.find(&graph, a, 0), Some(first));

        // Forwarding marks the projection dead; returning its replacement
        // would disagree with find_projection, even when a duplicate exists.
        graph.replace(first, b);
        assert_eq!(index.find(&graph, a, 0), Some(duplicate));
        graph.set_input(duplicate, 0, b);
        index.register(&graph, duplicate);
        assert_eq!(index.find(&graph, a, 0), graph.find_projection(a, 0));
        assert_eq!(index.find(&graph, b, 0), Some(duplicate));

        graph.set_opcode_payload(duplicate, Opcode::Projection, 1);
        index.register(&graph, duplicate);
        assert_eq!(index.find(&graph, b, 0), None);
        assert_eq!(index.find(&graph, b, 1), Some(duplicate));
        graph.set_opcode_payload(duplicate, Opcode::Region, 0);
        graph.set_inputs(duplicate, &[]);
        assert_eq!(index.find(&graph, b, 1), graph.find_projection(b, 1));

        let removed = projection(&mut graph, b, 1);
        index.register(&graph, removed);
        graph.remove_node(removed);
        assert_eq!(index.find(&graph, b, 1), None);
        let fresh = projection(&mut graph, b, 1);
        index.register(&graph, fresh);
        assert_eq!(index.find(&graph, b, 1), Some(fresh));

        // Source forwarding also keeps the raw-key contract until edges are
        // explicitly rewritten. Finalization requires discarding the index.
        graph.replace(b, a);
        assert_eq!(index.find(&graph, b, 1), graph.find_projection(b, 1));
        assert_eq!(index.find(&graph, a, 1), None);
        graph.resolve_edges();
        let mut rebuilt = ProjectionIndex::new(&graph);
        assert_eq!(rebuilt.find(&graph, b, 1), None);
        assert_eq!(rebuilt.find(&graph, a, 1), graph.find_projection(a, 1));
    }

    #[test]
    fn graph_growth_does_not_repeat_indexing_or_scan_unrelated_nodes() {
        let mut graph = graph();
        let mut index = ProjectionIndex::new(&graph);
        for _ in 0..512 {
            let source = source(&mut graph);
            for output in 0..2 {
                assert_eq!(index.find(&graph, source, output), None);
                let node = projection(&mut graph, source, output);
                index.register(&graph, node);
                assert_eq!(index.find(&graph, source, output), Some(node));
            }
        }
        let mut stats = LayoutStats::default();
        index.write_stats(&mut stats);
        assert_eq!(stats.projection_indexed, 0);
        assert_eq!(stats.projection_queries, 2048);
        assert_eq!(stats.projection_cache_misses, 1024);
        assert_eq!(stats.projection_candidates_checked, 1024);
    }
}
