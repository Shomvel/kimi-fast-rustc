use crate::son::{Graph, NodeKind};
use crate::{son_optimize, value_numbering};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct OptimizeStats {
    pub value_numbering_merges: usize,
    pub constants_folded: usize,
    pub algebraic_simplifications: usize,
    pub dead_nodes_removed: usize,
}

impl OptimizeStats {
    pub fn total_rewrites(self) -> usize {
        self.value_numbering_merges + self.constants_folded + self.algebraic_simplifications
    }

    pub fn rewrites(self) -> usize {
        self.total_rewrites()
    }
}

pub fn optimize(graph: &mut Graph) -> OptimizeStats {
    if graph.iter().any(|(_, node)| {
        matches!(
            node.kind,
            NodeKind::If | NodeKind::Projection(_) | NodeKind::Region
        )
    }) {
        return optimize_control_flow(graph);
    }

    let mut total = OptimizeStats::default();

    loop {
        let before_value_numbering = graph.clone();
        let reported_merges = value_numbering::value_numbering(graph);
        let merges = if graphs_equal(graph, &before_value_numbering) {
            0
        } else {
            reported_merges
        };
        let stats = son_optimize::optimize(graph);
        let strength = strength_reduce_control_graph(graph);

        total.value_numbering_merges += merges;
        total.constants_folded += stats.constants_folded;
        total.algebraic_simplifications += stats.algebraic_simplifications + strength;
        total.dead_nodes_removed += stats.dead_nodes_removed;

        if merges == 0 && stats.rewrites() == 0 && strength == 0 {
            return total;
        }
    }
}

fn optimize_control_flow(graph: &mut Graph) -> OptimizeStats {
    let mut total = OptimizeStats::default();
    loop {
        let before_value_numbering = graph.clone();
        let reported_merges = value_numbering::value_numbering(graph);
        let merges = if graphs_equal(graph, &before_value_numbering) {
            0
        } else {
            reported_merges
        };
        let folded = fold_control_constants(graph);
        let simplified = simplify_control_graph(graph) + strength_reduce_control_graph(graph);
        let removed = eliminate_control_dead_nodes(graph);
        total.value_numbering_merges += merges;
        total.constants_folded += folded;
        total.algebraic_simplifications += simplified;
        total.dead_nodes_removed += removed;
        if merges == 0 && folded == 0 && simplified == 0 && removed == 0 {
            return total;
        }
    }
}

fn fold_control_constants(graph: &mut Graph) -> usize {
    let mut folded = 0;
    for id in graph.ids().collect::<Vec<_>>() {
        let Some(node) = graph.node(id) else {
            continue;
        };
        if !node.control.is_empty()
            || !node.memory.is_empty()
            || !matches!(
                node.kind,
                NodeKind::Unary(_) | NodeKind::Binary(_) | NodeKind::Compare(_)
            )
        {
            continue;
        }
        let Some(value) = son_optimize::constant_value(graph, id) else {
            continue;
        };
        let node = graph.node_mut(id).expect("folded node exists");
        node.kind = NodeKind::Const(value);
        node.data.clear();
        folded += 1;
    }
    folded
}

fn eliminate_control_dead_nodes(graph: &mut Graph) -> usize {
    let roots = graph
        .iter()
        .filter_map(|(id, node)| {
            matches!(
                node.kind,
                NodeKind::Start
                    | NodeKind::If
                    | NodeKind::Projection(_)
                    | NodeKind::Region
                    | NodeKind::Jump
                    | NodeKind::Call(_)
                    | NodeKind::Return
                    | NodeKind::Binary(
                        crate::son::BinaryOp::Div
                            | crate::son::BinaryOp::Rem
                            | crate::son::BinaryOp::UDiv
                            | crate::son::BinaryOp::URem
                    )
            )
            .then_some(id)
        })
        .collect::<Vec<_>>();
    son_optimize::eliminate_dead_nodes_with_roots(graph, &roots)
}

fn strength_reduce_control_graph(graph: &mut Graph) -> usize {
    let mut reduced = 0;
    for id in graph.ids().collect::<Vec<_>>() {
        let Some(node) = graph.node(id) else {
            continue;
        };
        if !node.control.is_empty() || !node.memory.is_empty() {
            continue;
        }
        let NodeKind::Binary(operation) = node.kind else {
            continue;
        };
        let data = node.data.clone();
        match operation {
            crate::son::BinaryOp::Mul => {
                if let Some(shift) = node_const(graph, data[1]).and_then(power_of_two_shift) {
                    replace_binary_constant(graph, id, crate::son::BinaryOp::Shl, 1, shift);
                } else if let Some(shift) = node_const(graph, data[0]).and_then(power_of_two_shift)
                {
                    let node = graph.node_mut(id).expect("reduced node exists");
                    node.kind = NodeKind::Binary(crate::son::BinaryOp::Shl);
                    node.data = vec![data[1], data[0]];
                    replace_binary_constant(graph, id, crate::son::BinaryOp::Shl, 1, shift);
                } else {
                    continue;
                }
            }
            crate::son::BinaryOp::UDiv => {
                let Some(shift) = node_const(graph, data[1]).and_then(power_of_two_shift) else {
                    continue;
                };
                replace_binary_constant(graph, id, crate::son::BinaryOp::LShr, 1, shift);
            }
            crate::son::BinaryOp::URem => {
                let Some(divisor) = node_const(graph, data[1]) else {
                    continue;
                };
                if power_of_two_shift(divisor).is_none() {
                    continue;
                }
                replace_binary_constant(
                    graph,
                    id,
                    crate::son::BinaryOp::And,
                    1,
                    divisor.wrapping_sub(1),
                );
            }
            _ => continue,
        }
        reduced += 1;
    }
    reduced
}

fn replace_binary_constant(
    graph: &mut Graph,
    id: crate::son::NodeId,
    operation: crate::son::BinaryOp,
    index: usize,
    value: i64,
) {
    graph.node_mut(id).expect("reduced node exists").kind = NodeKind::Binary(operation);
    let constant = graph.constant(value);
    graph.node_mut(id).expect("reduced node exists").data[index] = constant;
}

fn node_const(graph: &Graph, id: crate::son::NodeId) -> Option<i64> {
    match graph.node(id)?.kind {
        NodeKind::Const(value) => Some(value),
        _ => None,
    }
}

fn power_of_two_shift(value: i64) -> Option<i64> {
    (value != 0 && value.count_ones() == 1).then_some(value.trailing_zeros() as i64)
}

fn simplify_control_graph(graph: &mut Graph) -> usize {
    let mut simplified = 0;
    for id in graph.ids().collect::<Vec<_>>() {
        let Some(node) = graph.node(id) else {
            continue;
        };
        if !node.control.is_empty()
            || !node.memory.is_empty()
            || !matches!(
                node.kind,
                NodeKind::Unary(_) | NodeKind::Binary(_) | NodeKind::Compare(_)
            )
        {
            continue;
        }
        simplified += usize::from(son_optimize::simplify_node_in_place(graph, id));
    }
    simplified
}

fn graphs_equal(left: &Graph, right: &Graph) -> bool {
    left.len() == right.len() && left.iter().eq(right.iter())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interpreter::{InterpretError, TrapKind, interpret};
    use crate::son::{BinaryOp, NodeKind};

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum Outcome {
        Value(i64),
        Trap(TrapKind),
    }

    fn evaluate(graph: &Graph, arguments: &[i64]) -> Outcome {
        match interpret(graph, arguments) {
            Ok(value) => Outcome::Value(value),
            Err(InterpretError::Trap { kind, .. }) => Outcome::Trap(kind),
            Err(error) => panic!("interpreter rejected test graph: {error}"),
        }
    }

    fn assert_differential<const N: usize>(
        original: &Graph,
        optimized: &Graph,
        cases: &[[i64; N]],
    ) {
        for arguments in cases {
            assert_eq!(
                evaluate(original, arguments),
                evaluate(optimized, arguments),
                "arguments: {arguments:?}"
            );
        }
    }

    #[test]
    fn repeated_expressions_match_interpreter_on_parameters_and_boundaries() {
        let mut original = Graph::new();
        let start = original.start();
        let parameter = original.parameter(0);
        let duplicate_parameter = original.parameter(0);
        let square = original.binary(BinaryOp::Mul, parameter, parameter);
        let duplicate_square =
            original.binary(BinaryOp::Mul, duplicate_parameter, duplicate_parameter);
        let result = original.binary(BinaryOp::Add, square, duplicate_square);
        original.return_(start, Some(result));

        let mut optimized = original.clone();
        let stats = optimize(&mut optimized);

        assert_eq!(stats.value_numbering_merges, 2);
        assert_eq!(stats.total_rewrites(), 2);
        assert_differential(
            &original,
            &optimized,
            &[[0], [1], [-1], [7], [i64::MAX], [i64::MIN]],
        );
        optimized.validate().unwrap();
    }

    #[test]
    fn commutative_expressions_match_interpreter_on_boundaries() {
        for operation in [
            BinaryOp::Add,
            BinaryOp::Mul,
            BinaryOp::And,
            BinaryOp::Or,
            BinaryOp::Xor,
        ] {
            let mut original = Graph::new();
            let start = original.start();
            let left = original.parameter(0);
            let right = original.parameter(1);
            original.binary(operation, left, right);
            let reversed = original.binary(operation, right, left);
            original.return_(start, Some(reversed));

            let mut optimized = original.clone();
            let stats = optimize(&mut optimized);

            assert_eq!(stats.value_numbering_merges, 1, "{operation:?}");
            assert_eq!(stats.rewrites(), 1, "{operation:?}");
            assert_differential(
                &original,
                &optimized,
                &[
                    [0, 0],
                    [1, 2],
                    [-1, 1],
                    [i64::MAX, 1],
                    [i64::MIN, -1],
                    [i64::MIN, i64::MAX],
                ],
            );
            optimized.validate().unwrap();
        }
    }

    #[test]
    fn alternates_until_folding_and_value_numbering_reach_a_fixed_point() {
        let mut graph = Graph::new();
        let start = graph.start();
        let forty = graph.constant(40);
        let two = graph.constant(2);
        let sum = graph.binary(BinaryOp::Add, forty, two);
        let six = graph.constant(6);
        let seven = graph.constant(7);
        let product = graph.binary(BinaryOp::Mul, six, seven);
        let zero = graph.constant(0);
        let first_division = graph.binary(BinaryOp::Div, sum, zero);
        let second_division = graph.binary(BinaryOp::Div, product, zero);
        let result = graph.return_(start, Some(second_division));

        let stats = optimize(&mut graph);

        assert_eq!(stats.value_numbering_merges, 2);
        assert_eq!(stats.constants_folded, 2);
        assert_eq!(stats.total_rewrites(), 4);
        assert_eq!(graph.node(result).unwrap().data, vec![first_division]);
        assert_eq!(
            graph
                .iter()
                .filter(|(_, node)| node.kind == NodeKind::Const(42))
                .count(),
            1
        );
        graph.validate().unwrap();

        let fixed_point = graph.clone();
        assert_eq!(optimize(&mut graph), OptimizeStats::default());
        assert!(graphs_equal(&graph, &fixed_point));
    }

    #[test]
    fn trapping_division_and_remainder_survive_without_repeated_merges() {
        for (operation, left, right) in [
            (BinaryOp::Div, 1, 0),
            (BinaryOp::Rem, 1, 0),
            (BinaryOp::Div, i64::MIN, -1),
            (BinaryOp::Rem, i64::MIN, -1),
        ] {
            let mut original = Graph::new();
            let start = original.start();
            let left = original.constant(left);
            let right = original.constant(right);
            let first = original.binary(operation, left, right);
            let duplicate = original.binary(operation, left, right);
            let result = original.return_(start, Some(duplicate));

            let mut optimized = original.clone();
            let stats = optimize(&mut optimized);

            assert_eq!(stats.value_numbering_merges, 1, "{operation:?}");
            assert_eq!(stats.total_rewrites(), 1, "{operation:?}");
            assert_eq!(optimized.node(result).unwrap().data, vec![first]);
            assert!(optimized.node(first).is_some());
            assert!(optimized.node(duplicate).is_some());
            assert_differential(&original, &optimized, &[[]]);
            optimized.validate().unwrap();

            let fixed_point = optimized.clone();
            assert_eq!(optimize(&mut optimized), OptimizeStats::default());
            assert!(graphs_equal(&optimized, &fixed_point));
        }
    }

    #[test]
    fn self_replacements_do_not_count_as_progress() {
        let mut graph = Graph::new();
        let start = graph.start();
        let parameter = graph.parameter(0);
        let self_reference = graph.binary(BinaryOp::And, parameter, parameter);
        graph.node_mut(self_reference).unwrap().data = vec![self_reference; 2];
        graph.return_(start, Some(self_reference));

        let stats = optimize(&mut graph);

        assert_eq!(stats.value_numbering_merges, 0);
        assert_eq!(stats.total_rewrites(), 0);
        assert!(graph.node(self_reference).is_some());
        graph.validate().unwrap();

        let fixed_point = graph.clone();
        assert_eq!(optimize(&mut graph), OptimizeStats::default());
        assert!(graphs_equal(&graph, &fixed_point));
    }
}
