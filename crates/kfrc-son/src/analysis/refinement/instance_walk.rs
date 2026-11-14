//! Test-only reference traversal for the former dynamic-instance check.
//! Production uses the dominance cuts; retain this independent loop model to
//! exercise invalid paths which the prerequisite anchor checks must reject.

use super::WitnessStats;
use crate::NodeId;
use crate::analysis::cfg::ControlFlow;

pub(super) struct InstanceWalk {
    node_count: usize,
    marks: Vec<u32>,
    generation: u32,
    pending: Vec<(NodeId, bool)>,
}

impl InstanceWalk {
    pub(super) fn new(node_count: usize) -> Self {
        Self {
            node_count,
            marks: Vec::new(),
            generation: 0,
            pending: Vec::new(),
        }
    }

    /// Traversing the proof exit refreshes evidence for a new dynamic value.
    /// Between refreshes, any source definition invalidates the old proof.
    pub(super) fn unchanged_until(
        &mut self,
        cfg: &ControlFlow,
        cyclic: bool,
        proof: NodeId,
        at: NodeId,
        anchors: &[NodeId],
        stats: &mut WitnessStats,
    ) -> bool {
        if anchors.is_empty() || proof == at || !cyclic {
            return true;
        }
        self.marks.resize(self.node_count, 0);
        // The low bits record clean/dirty incoming states independently. Never
        // reuse a generation until all marks carrying that generation are gone.
        self.generation = match self.generation.checked_add(4) {
            Some(next) => next,
            None => {
                self.marks.fill(0);
                4
            }
        };
        // A previous query may have returned false with successors still queued.
        self.pending.clear();
        self.pending
            .extend(cfg.successors(proof).iter().map(|&node| (node, false)));
        while let Some((node, dirty)) = self.pending.pop() {
            if node == proof {
                continue;
            }
            let mark = &mut self.marks[node.index()];
            let bit = 1 << u32::from(dirty);
            if *mark & !3 != self.generation {
                *mark = self.generation;
            }
            if *mark & bit != 0 {
                continue;
            }
            *mark |= bit;
            stats.node_visits += 1;
            let dirty = dirty || anchors.contains(&node);
            if node == at && dirty {
                return false;
            }
            self.pending
                .extend(cfg.successors(node).iter().map(|&next| (next, dirty)));
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{FunctionGraph, Opcode, ValueType};
    use kfrc_body::{FunctionAbi, PassMode, RecordedTypes};
    use std::collections::{BTreeSet, VecDeque};

    fn graph() -> (FunctionGraph, Vec<NodeId>) {
        let mut graph = FunctionGraph::new(
            "instance_paths",
            FunctionAbi {
                args: vec![],
                ret: PassMode::Ignore,
                c_variadic: false,
            },
            RecordedTypes::new(),
        );
        graph.start = graph.add_node(Opcode::Start, NodeId::NONE, &[], 0, ValueType::Control);
        let nodes: Vec<_> = (0..24)
            .map(|_| {
                // Value records interspersed with controls exercise sparse IDs.
                graph.add_node(Opcode::Undefined, NodeId::NONE, &[], 0, ValueType::Effect);
                graph.add_node(Opcode::Region, NodeId::NONE, &[], 0, ValueType::Control)
            })
            .collect();
        // The ring reaches every control and crosses both forward/back edges.
        // Extra chords create joins where clean and dirty states must coexist.
        for (index, &node) in nodes.iter().enumerate() {
            let mut inputs = vec![
                nodes[(index + nodes.len() - 1) % nodes.len()],
                nodes[(index * 7 + 3) % nodes.len()],
            ];
            if index == 0 {
                inputs.push(graph.start);
            }
            graph.set_inputs(node, &inputs);
        }
        (graph, nodes)
    }

    // Breadth-first reachability over the full product graph is independent of
    // this reference's stack, generation stamps and early failure behavior.
    fn reachable_without_redefinition(
        cfg: &ControlFlow,
        proof: NodeId,
        at: NodeId,
        anchors: &[NodeId],
    ) -> bool {
        if anchors.is_empty() || proof == at {
            return true;
        }
        let mut queue = VecDeque::from([(proof, false)]);
        let mut reached = BTreeSet::new();
        while let Some((node, dirty)) = queue.pop_front() {
            for &next in cfg.successors(node) {
                if next == proof {
                    continue;
                }
                let dirty = dirty || anchors.contains(&next);
                if reached.insert((next, dirty)) {
                    queue.push_back((next, dirty));
                }
            }
        }
        !reached.contains(&(at, true))
    }

    #[test]
    fn repeated_queries_match_product_graph_reachability() {
        let (graph, nodes) = graph();
        let cfg = ControlFlow::new(&graph);
        let mut walk = InstanceWalk::new(graph.len());
        let mut random = 0x1294_d425_bf45_d113u64;
        for _ in 0..2048 {
            random = random.wrapping_mul(6364136223846793005).wrapping_add(1);
            let proof = nodes[random as usize % nodes.len()];
            let at = nodes[(random >> 8) as usize % nodes.len()];
            let anchors = [
                nodes[(random >> 16) as usize % nodes.len()],
                nodes[(random >> 32) as usize % nodes.len()],
            ];
            let mut stats = WitnessStats::default();
            assert_eq!(
                walk.unchanged_until(&cfg, true, proof, at, &anchors, &mut stats),
                reachable_without_redefinition(&cfg, proof, at, &anchors),
                "proof {proof}, at {at}, anchors {anchors:?}"
            );
            assert!(stats.node_visits <= cfg.order.len() * 2);
        }
    }

    #[test]
    fn rollover_and_early_exit_do_not_keep_previous_query_states() {
        let (graph, nodes) = graph();
        let cfg = ControlFlow::new(&graph);
        let mut walk = InstanceWalk::new(graph.len());
        let proof = nodes[0];
        let at = nodes[1];
        let mut stats = WitnessStats::default();
        assert!(!walk.unchanged_until(&cfg, true, proof, at, &[at], &mut stats));
        walk.generation = u32::MAX & !3;
        assert!(!walk.unchanged_until(&cfg, true, proof, at, &[at], &mut stats));
        // The proof itself is a cut, so it cannot dirty the following interval.
        assert!(walk.unchanged_until(&cfg, true, proof, at, &[proof], &mut stats));
        assert!(!walk.unchanged_until(&cfg, true, proof, at, &[at], &mut stats));
    }

    #[test]
    fn fast_paths_do_not_allocate_traversal_storage() {
        let (graph, nodes) = graph();
        let cfg = ControlFlow::new(&graph);
        let mut walk = InstanceWalk::new(graph.len());
        let mut stats = WitnessStats::default();
        assert!(walk.unchanged_until(&cfg, true, nodes[0], nodes[1], &[], &mut stats));
        assert!(walk.unchanged_until(&cfg, true, nodes[0], nodes[0], &nodes, &mut stats));
        assert!(walk.unchanged_until(&cfg, false, nodes[0], nodes[1], &nodes, &mut stats));
        assert!(walk.marks.is_empty() && walk.pending.is_empty());
        assert_eq!(stats.node_visits, 0);
    }
}
