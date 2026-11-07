use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;

use crate::son::{Graph, InputChannel, NodeId, NodeKind, ValidationError};

const CHANNELS: [InputChannel; 3] = [
    InputChannel::Control,
    InputChannel::Data,
    InputChannel::Memory,
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum UnsupportedReason {
    NodeKind(NodeKind),
    MultipleStarts,
    MultipleReturns,
    ControlCycle,
    DependencyCycle,
    ReturnControlNotStart,
    ReturnHasUsers,
    MemoryMerge,
    MemoryBranch { producer: NodeId },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PlacementError {
    GraphValidation(ValidationError),
    MissingStart,
    MissingReturn,
    Unsupported {
        node: NodeId,
        reason: UnsupportedReason,
    },
}

impl fmt::Display for PlacementError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::GraphValidation(error) => write!(formatter, "invalid graph: {error}"),
            Self::MissingStart => formatter.write_str("graph has no start node"),
            Self::MissingReturn => formatter.write_str("graph has no return node"),
            Self::Unsupported { node, reason } => match reason {
                UnsupportedReason::NodeKind(kind) => {
                    write!(formatter, "unsupported node {node}: {kind:?}")
                }
                UnsupportedReason::MultipleStarts => {
                    write!(formatter, "unsupported node {node}: multiple start nodes")
                }
                UnsupportedReason::MultipleReturns => {
                    write!(formatter, "unsupported node {node}: multiple returns")
                }
                UnsupportedReason::ControlCycle => {
                    write!(
                        formatter,
                        "unsupported node {node}: control dependency cycle"
                    )
                }
                UnsupportedReason::DependencyCycle => {
                    write!(formatter, "unsupported node {node}: dependency cycle")
                }
                UnsupportedReason::ReturnControlNotStart => write!(
                    formatter,
                    "unsupported node {node}: return control is not the start node"
                ),
                UnsupportedReason::ReturnHasUsers => {
                    write!(
                        formatter,
                        "unsupported node {node}: return is used as an input"
                    )
                }
                UnsupportedReason::MemoryMerge => {
                    write!(formatter, "unsupported node {node}: merging memory states")
                }
                UnsupportedReason::MemoryBranch { producer } => write!(
                    formatter,
                    "unsupported node {node}: memory state {producer} has multiple users"
                ),
            },
        }
    }
}

impl Error for PlacementError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::GraphValidation(error) => Some(error),
            _ => None,
        }
    }
}

impl From<ValidationError> for PlacementError {
    fn from(error: ValidationError) -> Self {
        Self::GraphValidation(error)
    }
}

/// Rebuilds a valid single-block graph in stable semantic dependency order.
///
/// Ready nodes retain their original `NodeId` order. Memory dependencies are
/// accepted only when they form disjoint, non-branching chains.
pub fn place_graph(graph: &Graph) -> Result<Graph, PlacementError> {
    graph.validate()?;

    if let Some(node) = control_cycle(graph) {
        return Err(unsupported(node, UnsupportedReason::ControlCycle));
    }

    let mut start = None;
    let mut return_node = None;
    let mut memory_users = BTreeMap::new();

    for (id, node) in graph.iter() {
        match node.kind {
            NodeKind::Start => {
                if start.replace(id).is_some() {
                    return Err(unsupported(id, UnsupportedReason::MultipleStarts));
                }
            }
            NodeKind::Return => {
                if return_node.replace(id).is_some() {
                    return Err(unsupported(id, UnsupportedReason::MultipleReturns));
                }
            }
            NodeKind::Phi => {
                return Err(unsupported(id, UnsupportedReason::NodeKind(NodeKind::Phi)));
            }
            _ => {}
        }

        if node.memory.len() > 1 {
            return Err(unsupported(id, UnsupportedReason::MemoryMerge));
        }
        for &producer in &node.memory {
            if memory_users.insert(producer, id).is_some() {
                return Err(unsupported(
                    id,
                    UnsupportedReason::MemoryBranch { producer },
                ));
            }
        }
    }

    let start = start.ok_or(PlacementError::MissingStart)?;
    let return_node = return_node.ok_or(PlacementError::MissingReturn)?;
    if graph
        .node(return_node)
        .expect("return was found")
        .control
        .as_slice()
        != [start]
    {
        return Err(unsupported(
            return_node,
            UnsupportedReason::ReturnControlNotStart,
        ));
    }

    for (_, node) in graph.iter() {
        if CHANNELS
            .into_iter()
            .any(|channel| node.inputs(channel).contains(&return_node))
        {
            return Err(unsupported(return_node, UnsupportedReason::ReturnHasUsers));
        }
    }

    let mut indegree: BTreeMap<NodeId, usize> = graph.ids().map(|id| (id, 0)).collect();
    let mut dependents: BTreeMap<NodeId, Vec<NodeId>> =
        graph.ids().map(|id| (id, Vec::new())).collect();
    for (id, node) in graph.iter() {
        for channel in CHANNELS {
            for &input in node.inputs(channel) {
                *indegree.get_mut(&id).expect("node was collected") += 1;
                dependents
                    .get_mut(&input)
                    .expect("validation checked the input")
                    .push(id);
            }
        }
    }

    let mut ready: BTreeSet<NodeId> = indegree
        .iter()
        .filter_map(|(&id, &count)| (count == 0 && id != start && id != return_node).then_some(id))
        .collect();
    let mut order = Vec::with_capacity(graph.len());
    order.push(start);
    release_dependents(start, return_node, &dependents, &mut indegree, &mut ready);

    while let Some(id) = ready.pop_first() {
        order.push(id);
        release_dependents(id, return_node, &dependents, &mut indegree, &mut ready);
    }

    if order.len() + 1 != graph.len() || indegree[&return_node] != 0 {
        let node = indegree
            .iter()
            .find_map(|(&id, &count)| (count != 0).then_some(id))
            .expect("an unplaced node has an unsatisfied dependency");
        return Err(unsupported(node, UnsupportedReason::DependencyCycle));
    }
    order.push(return_node);

    let mut placed = Graph::new();
    let mut mapping = BTreeMap::new();
    for old_id in order {
        let mut node = graph.node(old_id).expect("ordered node exists").clone();
        for channel in CHANNELS {
            for input in node.inputs_mut(channel) {
                *input = mapping[&*input];
            }
        }
        let new_id = placed.push(node);
        mapping.insert(old_id, new_id);
    }

    debug_assert!(placed.validate().is_ok());
    Ok(placed)
}

fn release_dependents(
    id: NodeId,
    return_node: NodeId,
    dependents: &BTreeMap<NodeId, Vec<NodeId>>,
    indegree: &mut BTreeMap<NodeId, usize>,
    ready: &mut BTreeSet<NodeId>,
) {
    for &dependent in &dependents[&id] {
        let count = indegree
            .get_mut(&dependent)
            .expect("dependent was collected");
        *count -= 1;
        if *count == 0 && dependent != return_node {
            ready.insert(dependent);
        }
    }
}

fn control_cycle(graph: &Graph) -> Option<NodeId> {
    let mut indegree: BTreeMap<NodeId, usize> = graph
        .ids()
        .map(|id| (id, graph.node(id).expect("ID exists").control.len()))
        .collect();
    let mut dependents: BTreeMap<NodeId, Vec<NodeId>> =
        graph.ids().map(|id| (id, Vec::new())).collect();
    for (id, node) in graph.iter() {
        for &input in &node.control {
            dependents
                .get_mut(&input)
                .expect("validation checked the input")
                .push(id);
        }
    }

    let mut ready: BTreeSet<NodeId> = indegree
        .iter()
        .filter_map(|(&id, &count)| (count == 0).then_some(id))
        .collect();
    let mut visited = 0;
    while let Some(id) = ready.pop_first() {
        visited += 1;
        for &dependent in &dependents[&id] {
            let count = indegree
                .get_mut(&dependent)
                .expect("dependent was collected");
            *count -= 1;
            if *count == 0 {
                ready.insert(dependent);
            }
        }
    }

    (visited != graph.len())
        .then(|| {
            indegree
                .iter()
                .find_map(|(&id, &count)| (count != 0).then_some(id))
        })
        .flatten()
}

fn unsupported(node: NodeId, reason: UnsupportedReason) -> PlacementError {
    PlacementError::Unsupported { node, reason }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native::{CompileError, lower_graph};
    use crate::son::{BinaryOp, Node};
    use crate::son_optimize::optimize;

    fn assert_dependency_order(graph: &Graph) {
        graph.validate().unwrap();
        let ids: Vec<_> = graph.ids().collect();
        assert_eq!(graph.node(ids[0]).unwrap().kind, NodeKind::Start);
        assert_eq!(
            graph.node(*ids.last().unwrap()).unwrap().kind,
            NodeKind::Return
        );
        let positions: BTreeMap<_, _> = ids
            .iter()
            .enumerate()
            .map(|(position, &id)| (id, position))
            .collect();
        for (id, node) in graph.iter() {
            for channel in CHANNELS {
                for input in node.inputs(channel) {
                    assert!(positions[input] < positions[&id]);
                }
            }
        }
    }

    #[test]
    fn places_constants_appended_by_optimization_before_return() {
        let mut graph = Graph::new();
        let start = graph.start();
        let left = graph.constant(40);
        let right = graph.constant(2);
        let folded = graph.binary(BinaryOp::Add, left, right);
        let result = graph.return_(start, Some(folded));

        optimize(&mut graph);
        let appended = graph.node(result).unwrap().data[0];
        assert!(appended > result);
        assert!(matches!(
            lower_graph(&graph),
            Err(CompileError::InvalidValueReference { node, .. }) if node == result
        ));

        let placed = place_graph(&graph).unwrap();
        assert_dependency_order(&placed);
        assert_eq!(placed.len(), 3);
        lower_graph(&placed).unwrap();
    }

    #[test]
    fn places_ordinary_arithmetic_with_stable_ready_order() {
        let mut graph = Graph::new();
        let start = graph.start();
        graph.add_node(NodeKind::Return, vec![start], vec![NodeId(2)]);
        graph.add_node(
            NodeKind::Binary(BinaryOp::Add),
            Vec::new(),
            vec![NodeId(3), NodeId(4)],
        );
        graph.constant(20);
        graph.constant(22);

        let placed = place_graph(&graph).unwrap();
        assert_dependency_order(&placed);
        let kinds: Vec<_> = placed.iter().map(|(_, node)| node.kind).collect();
        assert_eq!(
            kinds,
            vec![
                NodeKind::Start,
                NodeKind::Const(20),
                NodeKind::Const(22),
                NodeKind::Binary(BinaryOp::Add),
                NodeKind::Return,
            ]
        );
        lower_graph(&placed).unwrap();
    }

    #[test]
    fn start_is_the_entry_even_when_its_original_id_is_later() {
        let mut graph = Graph::new();
        graph.constant(7);
        let start = graph.start();
        graph.return_(start, Some(NodeId(0)));

        let placed = place_graph(&graph).unwrap();
        assert_dependency_order(&placed);
        assert_eq!(placed.node(NodeId(0)).unwrap().kind, NodeKind::Start);
        assert_eq!(placed.node(NodeId(1)).unwrap().kind, NodeKind::Const(7));
    }

    #[test]
    fn remaps_control_data_and_linear_memory_edges() {
        let mut graph = Graph::new();
        let start = graph.start();
        graph.push(Node::with_memory_inputs(
            NodeKind::Return,
            vec![start],
            vec![NodeId(2)],
            vec![NodeId(2)],
        ));
        graph.push(Node::with_memory_inputs(
            NodeKind::Binary(BinaryOp::Add),
            Vec::new(),
            vec![NodeId(3), NodeId(3)],
            vec![NodeId(3)],
        ));
        graph.constant(21);

        let placed = place_graph(&graph).unwrap();
        assert_dependency_order(&placed);
        let addition = placed.node(NodeId(2)).unwrap();
        assert_eq!(addition.data, vec![NodeId(1), NodeId(1)]);
        assert_eq!(addition.memory, vec![NodeId(1)]);
        let result = placed.node(NodeId(3)).unwrap();
        assert_eq!(result.control, vec![NodeId(0)]);
        assert_eq!(result.data, vec![NodeId(2)]);
        assert_eq!(result.memory, vec![NodeId(2)]);
    }

    #[test]
    fn rejects_phi_multiple_returns_and_control_cycles() {
        let mut graph = Graph::new();
        let start = graph.start();
        let value = graph.constant(1);
        graph.phi(vec![start], vec![value]);
        graph.return_(start, Some(value));
        assert!(matches!(
            place_graph(&graph),
            Err(PlacementError::Unsupported {
                reason: UnsupportedReason::NodeKind(NodeKind::Phi),
                ..
            })
        ));

        let mut graph = Graph::new();
        let start = graph.start();
        let value = graph.constant(1);
        graph.return_(start, Some(value));
        let second = graph.return_(start, Some(value));
        assert_eq!(
            place_graph(&graph).unwrap_err(),
            unsupported(second, UnsupportedReason::MultipleReturns)
        );

        let mut graph = Graph::new();
        graph.start();
        let result = graph.add_node(NodeKind::Return, vec![NodeId(1)], vec![NodeId(2)]);
        graph.constant(1);
        assert_eq!(
            place_graph(&graph).unwrap_err(),
            unsupported(result, UnsupportedReason::ControlCycle)
        );
    }

    #[test]
    fn rejects_dependency_cycles_and_return_users() {
        let mut graph = Graph::new();
        let start = graph.start();
        let one = graph.constant(1);
        graph.add_node(
            NodeKind::Binary(BinaryOp::Add),
            Vec::new(),
            vec![NodeId(3), one],
        );
        graph.add_node(
            NodeKind::Binary(BinaryOp::Add),
            Vec::new(),
            vec![NodeId(2), one],
        );
        graph.return_(start, Some(NodeId(2)));
        assert!(matches!(
            place_graph(&graph),
            Err(PlacementError::Unsupported {
                reason: UnsupportedReason::DependencyCycle,
                ..
            })
        ));

        let mut graph = Graph::new();
        let start = graph.start();
        let value = graph.constant(1);
        let result = graph.return_(start, Some(value));
        graph.binary(BinaryOp::Add, result, value);
        assert_eq!(
            place_graph(&graph).unwrap_err(),
            unsupported(result, UnsupportedReason::ReturnHasUsers)
        );
    }

    #[test]
    fn rejects_branching_merging_and_cyclic_memory() {
        let mut graph = Graph::new();
        let start = graph.start();
        let token = graph.constant(1);
        let first = graph.add_node_with_memory(
            NodeKind::Binary(BinaryOp::Add),
            Vec::new(),
            vec![token, token],
            vec![token],
        );
        let second = graph.add_node_with_memory(
            NodeKind::Binary(BinaryOp::Add),
            Vec::new(),
            vec![token, token],
            vec![token],
        );
        graph.return_(start, Some(first));
        assert_eq!(
            place_graph(&graph).unwrap_err(),
            unsupported(second, UnsupportedReason::MemoryBranch { producer: token })
        );

        let mut graph = Graph::new();
        let start = graph.start();
        let token = graph.constant(1);
        let result = graph.add_node_with_memory(
            NodeKind::Return,
            vec![start],
            vec![token],
            vec![token, token],
        );
        assert_eq!(
            place_graph(&graph).unwrap_err(),
            unsupported(result, UnsupportedReason::MemoryMerge)
        );

        let mut graph = Graph::new();
        let start = graph.start();
        let value = graph.constant(1);
        graph.add_node_with_memory(
            NodeKind::Binary(BinaryOp::Add),
            Vec::new(),
            vec![value, value],
            vec![NodeId(3)],
        );
        graph.add_node_with_memory(
            NodeKind::Binary(BinaryOp::Add),
            Vec::new(),
            vec![value, value],
            vec![NodeId(2)],
        );
        graph.return_(start, Some(NodeId(2)));
        assert!(matches!(
            place_graph(&graph),
            Err(PlacementError::Unsupported {
                reason: UnsupportedReason::DependencyCycle,
                ..
            })
        ));
    }

    #[test]
    fn rejects_invalid_and_incomplete_graphs() {
        let mut graph = Graph::new();
        let start = graph.start();
        graph.binary(BinaryOp::Add, NodeId(99), NodeId(99));
        graph.return_(start, Some(NodeId(1)));
        assert!(matches!(
            place_graph(&graph),
            Err(PlacementError::GraphValidation(
                ValidationError::MissingInput { .. }
            ))
        ));

        assert_eq!(
            place_graph(&Graph::new()).unwrap_err(),
            PlacementError::MissingStart
        );

        let mut graph = Graph::new();
        graph.start();
        assert_eq!(
            place_graph(&graph).unwrap_err(),
            PlacementError::MissingReturn
        );
    }
}
