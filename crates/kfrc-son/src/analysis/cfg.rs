//! The control skeleton shared by transforms and the verifier.
use crate::graph::{FunctionGraph, NodeId, Opcode, ValueType};
use std::collections::{HashMap, HashSet};

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

#[derive(Clone, Debug)]
pub struct ControlFlow {
    pub order: Vec<NodeId>,
    pub position: HashMap<NodeId, usize>,
    pub predecessors: Vec<Vec<usize>>,
    successors: HashMap<NodeId, Vec<NodeId>>,
}

impl ControlFlow {
    /// Requires existing operand references, but accepts unreachable control
    /// while a CFG edit is removing a branch. Dead sources have no successors.
    pub fn new(graph: &FunctionGraph) -> Self {
        let mut successors: HashMap<NodeId, Vec<NodeId>> = HashMap::new();
        let mut edge = |source: NodeId, target: NodeId| {
            let source = graph.resolve(source);
            if !source.is_none() && !graph.is_dead(source) {
                successors.entry(source).or_default().push(target);
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
        let mut order = Vec::new();
        let mut visited = HashSet::new();
        let mut stack = vec![(graph.start, 0usize)];
        visited.insert(graph.start);
        while let Some((node, next)) = stack.pop() {
            let list = successors.get(&node).map(Vec::as_slice).unwrap_or(&[]);
            if next < list.len() {
                stack.push((node, next + 1));
                let successor = list[next];
                if visited.insert(successor) {
                    stack.push((successor, 0));
                }
            } else {
                order.push(node);
            }
        }
        order.reverse();
        let position: HashMap<_, _> = order
            .iter()
            .enumerate()
            .map(|(index, node)| (*node, index))
            .collect();
        let mut predecessors = vec![Vec::new(); order.len()];
        for (index, node) in order.iter().enumerate() {
            for successor in successors.get(node).into_iter().flatten() {
                predecessors[position[successor]].push(index);
            }
        }
        Self {
            order,
            position,
            predecessors,
            successors,
        }
    }

    pub fn contains(&self, node: NodeId) -> bool {
        self.position.contains_key(&node)
    }
    pub fn successors(&self, node: NodeId) -> &[NodeId] {
        self.successors.get(&node).map(Vec::as_slice).unwrap_or(&[])
    }
}
