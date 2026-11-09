//! The control skeleton shared by transforms and the verifier.
use crate::graph::{FunctionGraph, NodeId, Opcode, ValueType};

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

#[derive(Clone, Debug)]
pub struct ControlFlow {
    pub order: Vec<NodeId>,
    pub position: NodePositions,
    pub predecessors: Vec<Vec<usize>>,
    /// Successors of node `n` are `successor_list[successor_starts[n]..successor_starts[n + 1]]`.
    successor_starts: Vec<u32>,
    successor_list: Vec<NodeId>,
}

impl ControlFlow {
    /// Requires existing operand references, but accepts unreachable control
    /// while a CFG edit is removing a branch. Dead sources have no successors.
    pub fn new(graph: &FunctionGraph) -> Self {
        // Edges in discovery order, grouped by source with a counting sort.
        let mut edges: Vec<(NodeId, NodeId)> = Vec::new();
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
        let mut predecessors = vec![Vec::new(); order.len()];
        for (index, node) in order.iter().enumerate() {
            for successor in successors(*node) {
                predecessors[positions[successor.index()]].push(index);
            }
        }
        Self {
            order,
            position: NodePositions { positions },
            predecessors,
            successor_starts,
            successor_list,
        }
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
