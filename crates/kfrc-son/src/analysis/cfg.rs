//! The control skeleton shared by transforms and the verifier.
use crate::graph::{FunctionGraph, NodeId, Opcode, ValueType};
use std::sync::{Arc, OnceLock};

use super::dominators::Dominators;

pub fn is_anchored_arithmetic(op: Opcode) -> bool {
    matches!(
        op,
        Opcode::DivSigned | Opcode::DivUnsigned | Opcode::RemSigned | Opcode::RemUnsigned
    )
}

/// Fixed value operations execute at their anchor without producing a new
/// control edge. Their operands and all uses remain below that anchor.
pub fn is_anchored_value(op: Opcode) -> bool {
    is_anchored_arithmetic(op)
}

#[inline]
pub fn is_control_flow(graph: &FunctionGraph, node: NodeId) -> bool {
    let op = graph.op(node);
    (op.is_fixed() && !is_anchored_value(op))
        || (op == Opcode::Projection && matches!(graph.value_type(node), ValueType::Control))
}

/// Position of each reachable control node in [`ControlFlow::order`],
/// indexed densely by node id; unreachable and value nodes have none.
/// Keeps the `get`/`contains_key`/`[&node]` interface of the map it replaces.
#[derive(Clone, Debug)]
pub struct NodePositions {
    positions: Vec<usize>,
}

impl NodePositions {
    const ABSENT: usize = usize::MAX;

    pub fn get(&self, node: &NodeId) -> Option<&usize> {
        self.positions
            .get(node.index())
            .filter(|position| **position != Self::ABSENT)
    }

    pub fn contains_key(&self, node: &NodeId) -> bool {
        self.get(node).is_some()
    }
}

impl std::ops::Index<&NodeId> for NodePositions {
    type Output = usize;

    fn index(&self, node: &NodeId) -> &usize {
        self.get(node).expect("node is not in the control order")
    }
}

/// Predecessor positions of every reachable control node, indexed by its
/// position in [`ControlFlow::order`]: one flat list grouped by target.
#[derive(Clone, Debug)]
pub struct Predecessors {
    starts: Vec<u32>,
    list: Vec<usize>,
}

impl Predecessors {
    pub fn len(&self) -> usize {
        self.starts.len() - 1
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn iter(&self) -> impl ExactSizeIterator<Item = &[usize]> + '_ {
        (0..self.len()).map(|position| &self[position])
    }
}

impl std::ops::Index<usize> for Predecessors {
    type Output = [usize];

    fn index(&self, position: usize) -> &[usize] {
        &self.list[self.starts[position] as usize..self.starts[position + 1] as usize]
    }
}

#[derive(Clone, Debug)]
pub struct ControlFlow {
    data: Arc<ControlFlowData>,
}

/// Read-only storage shared by snapshots of the same unmodified graph.
/// `ControlFlow` exposes these fields by shared dereference; editing them
/// would detach the order from its successor and dominance information.
#[derive(Debug)]
pub struct ControlFlowData {
    pub order: Vec<NodeId>,
    pub position: NodePositions,
    pub predecessors: Predecessors,
    /// Successors of node `n` are `successor_list[successor_starts[n]..successor_starts[n + 1]]`.
    successor_starts: Vec<u32>,
    successor_list: Vec<NodeId>,
    start: NodeId,
    dominators: OnceLock<Dominators>,
}

impl std::ops::Deref for ControlFlow {
    type Target = ControlFlowData;

    fn deref(&self) -> &Self::Target {
        &self.data
    }
}

impl ControlFlow {
    /// Requires existing operand references, but accepts unreachable control
    /// while a CFG edit is removing a branch. Dead sources have no successors.
    pub fn new(graph: &FunctionGraph) -> Self {
        graph.control_flow()
    }

    pub(crate) fn build(graph: &FunctionGraph) -> Self {
        // Edges in discovery order, grouped by source with a counting sort.
        let mut edges: Vec<(NodeId, NodeId)> = Vec::with_capacity(graph.len() / 2);
        let mut edge = |source: NodeId, target: NodeId| {
            let source = graph.resolve(source);
            if !source.is_none() && !graph.is_dead(source) {
                edges.push((source, target));
            }
        };
        for node in graph.ids().filter(|node| is_control_flow(graph, *node)) {
            edge(graph.control(node), node);
            match graph.op(node) {
                Opcode::Projection => edge(graph.input(node, 0), node),
                Opcode::Region => {
                    for &jump in graph.inputs(node) {
                        edge(jump, node);
                    }
                }
                _ => {}
            }
        }
        let mut successor_starts = vec![0u32; graph.len() + 1];
        for (source, _) in &edges {
            successor_starts[source.index() + 1] += 1;
        }
        for index in 1..successor_starts.len() {
            successor_starts[index] += successor_starts[index - 1];
        }
        let mut next = successor_starts.clone();
        let mut successor_list = vec![NodeId::NONE; edges.len()];
        for (source, target) in edges {
            let slot = &mut next[source.index()];
            successor_list[*slot as usize] = target;
            *slot += 1;
        }
        let successors = |node: NodeId| {
            let start = successor_starts[node.index()] as usize;
            &successor_list[start..successor_starts[node.index() + 1] as usize]
        };
        let mut order = Vec::new();
        let mut visited = vec![false; graph.len()];
        // A graph without a start node yet (still being built) has no flow.
        let mut stack = Vec::new();
        if graph.start.index() < graph.len() {
            stack.push((graph.start, 0usize));
            visited[graph.start.index()] = true;
        }
        while let Some((node, next)) = stack.pop() {
            let list = successors(node);
            if next < list.len() {
                stack.push((node, next + 1));
                let successor = list[next];
                if !visited[successor.index()] {
                    visited[successor.index()] = true;
                    stack.push((successor, 0));
                }
            } else {
                order.push(node);
            }
        }
        order.reverse();
        let mut positions = vec![NodePositions::ABSENT; graph.len()];
        for (index, node) in order.iter().enumerate() {
            positions[node.index()] = index;
        }
        // Same order as pushing `index` onto per-target lists while walking
        // `order` and each node's successors: a counting sort by target.
        let mut predecessor_starts = vec![0u32; order.len() + 1];
        for node in &order {
            for successor in successors(*node) {
                predecessor_starts[positions[successor.index()] + 1] += 1;
            }
        }
        for index in 1..predecessor_starts.len() {
            predecessor_starts[index] += predecessor_starts[index - 1];
        }
        let mut next = predecessor_starts.clone();
        let mut predecessor_list = vec![0usize; predecessor_starts[order.len()] as usize];
        for (index, node) in order.iter().enumerate() {
            for successor in successors(*node) {
                let slot = &mut next[positions[successor.index()]];
                predecessor_list[*slot as usize] = index;
                *slot += 1;
            }
        }
        let predecessors = Predecessors {
            starts: predecessor_starts,
            list: predecessor_list,
        };
        Self {
            data: Arc::new(ControlFlowData {
                order,
                position: NodePositions { positions },
                predecessors,
                successor_starts,
                successor_list,
                start: graph.start,
                dominators: OnceLock::new(),
            }),
        }
    }

    pub(crate) fn starts_at(&self, start: NodeId) -> bool {
        self.start == start
    }

    pub(super) fn dominators(&self) -> Dominators {
        self.dominators
            .get_or_init(|| Dominators::build(self))
            .clone()
    }

    pub fn contains(&self, node: NodeId) -> bool {
        self.position.contains_key(&node)
    }
    pub fn successors(&self, node: NodeId) -> &[NodeId] {
        match self.successor_starts.get(node.index() + 1) {
            Some(&end) => {
                &self.successor_list[self.successor_starts[node.index()] as usize..end as usize]
            }
            None => &[],
        }
    }
}

#[cfg(test)]
#[path = "cfg_tests.rs"]
mod tests;
