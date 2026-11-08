use std::collections::HashSet;

use crate::son::{BinaryOp, CompareOp, Graph, InputChannel, Node, NodeId, NodeKind, UnaryOp};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct OptimizeStats {
    pub constants_folded: usize,
    pub algebraic_simplifications: usize,
    pub dead_nodes_removed: usize,
}

impl OptimizeStats {
    pub fn rewrites(self) -> usize {
        self.constants_folded + self.algebraic_simplifications
    }
}

pub fn optimize(graph: &mut Graph) -> OptimizeStats {
    let mut stats = OptimizeStats::default();
    loop {
        let folded = constant_fold(graph);
        let simplified = algebraic_simplify(graph);
        stats.constants_folded += folded;
        stats.algebraic_simplifications += simplified;
        stats.dead_nodes_removed += eliminate_dead_nodes(graph);
        if folded == 0 && simplified == 0 {
            break;
        }
    }
    stats
}

pub fn constant_fold(graph: &mut Graph) -> usize {
    let ids: Vec<_> = graph.ids().collect();
    let mut folded = 0;
    for id in ids {
        let Some(value) = constant_value(graph, id) else {
            continue;
        };
        let replacement = graph.constant(value);
        graph.replace_uses(id, replacement);
        folded += 1;
    }
    folded
}

pub fn algebraic_simplify(graph: &mut Graph) -> usize {
    let ids: Vec<_> = graph.ids().collect();
    let mut simplified = 0;
    for id in ids {
        let Some(rewrite) = algebraic_rewrite(graph, id) else {
            continue;
        };
        match rewrite {
            Rewrite::Node(replacement) => {
                if replacement != id {
                    graph.replace_uses(id, replacement);
                    simplified += 1;
                }
            }
            Rewrite::Constant(value) => {
                let replacement = graph.constant(value);
                graph.replace_uses(id, replacement);
                simplified += 1;
            }
            Rewrite::Comparison {
                operation,
                left,
                right,
            } => {
                let node = graph.node_mut(id).expect("simplified node exists");
                node.kind = NodeKind::Compare(operation);
                node.data = vec![left, right];
                simplified += 1;
            }
        }
    }
    simplified
}

pub fn eliminate_dead_nodes(graph: &mut Graph) -> usize {
    eliminate_dead_nodes_with_roots(graph, &[])
}

pub fn eliminate_dead_nodes_with_roots(graph: &mut Graph, extra_roots: &[NodeId]) -> usize {
    let mut live = HashSet::new();
    let mut stack: Vec<_> = graph
        .iter()
        .filter_map(|(id, node)| is_root(graph, id, node).then_some(id))
        .chain(extra_roots.iter().copied())
        .collect();

    while let Some(id) = stack.pop() {
        if graph.node(id).is_none() || !live.insert(id) {
            continue;
        }
        let node = graph.node(id).expect("live node disappeared");
        for channel in [
            InputChannel::Control,
            InputChannel::Data,
            InputChannel::Memory,
        ] {
            stack.extend_from_slice(node.inputs(channel));
        }
    }

    let dead: Vec<_> = graph.ids().filter(|id| !live.contains(id)).collect();
    let removed = dead.len();
    for id in dead {
        graph.remove(id);
    }
    removed
}

#[derive(Clone, Copy)]
enum Rewrite {
    Node(NodeId),
    Constant(i64),
    Comparison {
        operation: CompareOp,
        left: NodeId,
        right: NodeId,
    },
}

fn const_value(graph: &Graph, id: NodeId) -> Option<i64> {
    match graph.node(id)?.kind {
        NodeKind::Const(value) => Some(value),
        _ => None,
    }
}

pub fn constant_value(graph: &Graph, id: NodeId) -> Option<i64> {
    let node = graph.node(id)?;
    match node.kind {
        NodeKind::Unary(op) if node.data.len() == 1 => {
            let value = const_value(graph, node.data[0])?;
            Some(match op {
                UnaryOp::Neg => value.wrapping_neg(),
                UnaryOp::Not => !value,
            })
        }
        NodeKind::Binary(op) if node.data.len() == 2 => {
            let left = const_value(graph, node.data[0])?;
            let right = const_value(graph, node.data[1])?;
            fold_binary(op, left, right)
        }
        NodeKind::Compare(op) if node.data.len() == 2 => {
            let left = const_value(graph, node.data[0])?;
            let right = const_value(graph, node.data[1])?;
            Some(bool_value(match op {
                CompareOp::Eq => left == right,
                CompareOp::Ne => left != right,
                CompareOp::Lt => left < right,
                CompareOp::Le => left <= right,
                CompareOp::Gt => left > right,
                CompareOp::Ge => left >= right,
                CompareOp::ULt => (left as u64) < right as u64,
                CompareOp::ULe => (left as u64) <= right as u64,
                CompareOp::UGt => (left as u64) > right as u64,
                CompareOp::UGe => (left as u64) >= right as u64,
            }))
        }
        _ => None,
    }
}

fn fold_binary(op: BinaryOp, left: i64, right: i64) -> Option<i64> {
    match op {
        BinaryOp::Add => Some(left.wrapping_add(right)),
        BinaryOp::Sub => Some(left.wrapping_sub(right)),
        BinaryOp::Mul => Some(left.wrapping_mul(right)),
        BinaryOp::Div => left.checked_div(right),
        BinaryOp::Rem => left.checked_rem(right),
        BinaryOp::UDiv => (left as u64)
            .checked_div(right as u64)
            .map(|value| value as i64),
        BinaryOp::URem => (left as u64)
            .checked_rem(right as u64)
            .map(|value| value as i64),
        BinaryOp::And => Some(left & right),
        BinaryOp::Or => Some(left | right),
        BinaryOp::Xor => Some(left ^ right),
        BinaryOp::Shl => Some(left.wrapping_shl(right as u32)),
        BinaryOp::Shr => Some(left.wrapping_shr(right as u32)),
        BinaryOp::LShr => Some((left as u64).wrapping_shr(right as u32) as i64),
    }
}

fn algebraic_rewrite(graph: &Graph, id: NodeId) -> Option<Rewrite> {
    let node = graph.node(id)?;
    match node.kind {
        NodeKind::Unary(op) if node.data.len() == 1 => {
            let inner_id = node.data[0];
            let inner = graph.node(inner_id)?;
            if inner.kind == NodeKind::Unary(op) && inner.data.len() == 1 {
                Some(Rewrite::Node(inner.data[0]))
            } else {
                None
            }
        }
        NodeKind::Binary(op) if node.data.len() == 2 => {
            simplify_binary(graph, op, node.data[0], node.data[1])
        }
        NodeKind::Compare(op) if node.data.len() == 2 => {
            if let Some(rewrite) =
                simplify_boolean_comparison(graph, op, node.data[0], node.data[1])
            {
                Some(rewrite)
            } else if node.data[0] == node.data[1] {
                let value = match op {
                    CompareOp::Eq
                    | CompareOp::Le
                    | CompareOp::Ge
                    | CompareOp::ULe
                    | CompareOp::UGe => 1,
                    CompareOp::Ne
                    | CompareOp::Lt
                    | CompareOp::Gt
                    | CompareOp::ULt
                    | CompareOp::UGt => 0,
                };
                Some(Rewrite::Constant(value))
            } else {
                None
            }
        }
        NodeKind::Phi if !node.data.is_empty() => {
            let replacement = node.data.iter().copied().find(|&input| input != id)?;
            node.data
                .iter()
                .all(|&input| input == id || input == replacement)
                .then_some(Rewrite::Node(replacement))
        }
        _ => None,
    }
}

fn simplify_boolean_comparison(
    graph: &Graph,
    outer: CompareOp,
    left: NodeId,
    right: NodeId,
) -> Option<Rewrite> {
    if !matches!(outer, CompareOp::Eq | CompareOp::Ne) {
        return None;
    }
    let (inner_id, constant) = if matches!(
        graph.node(right).map(|node| node.kind),
        Some(NodeKind::Const(0) | NodeKind::Const(1))
    ) {
        (left, const_value(graph, right)?)
    } else if matches!(
        graph.node(left).map(|node| node.kind),
        Some(NodeKind::Const(0) | NodeKind::Const(1))
    ) {
        (right, const_value(graph, left)?)
    } else {
        return None;
    };
    let inner = graph.node(inner_id)?;
    let NodeKind::Compare(inner_op) = inner.kind else {
        return None;
    };
    if inner.data.len() != 2 {
        return None;
    }
    let negate =
        (outer == CompareOp::Eq && constant == 0) || (outer == CompareOp::Ne && constant == 1);
    if negate {
        Some(Rewrite::Comparison {
            operation: inverse_comparison(inner_op),
            left: inner.data[0],
            right: inner.data[1],
        })
    } else {
        Some(Rewrite::Node(inner_id))
    }
}

fn inverse_comparison(operation: CompareOp) -> CompareOp {
    match operation {
        CompareOp::Eq => CompareOp::Ne,
        CompareOp::Ne => CompareOp::Eq,
        CompareOp::Lt => CompareOp::Ge,
        CompareOp::Le => CompareOp::Gt,
        CompareOp::Gt => CompareOp::Le,
        CompareOp::Ge => CompareOp::Lt,
        CompareOp::ULt => CompareOp::UGe,
        CompareOp::ULe => CompareOp::UGt,
        CompareOp::UGt => CompareOp::ULe,
        CompareOp::UGe => CompareOp::ULt,
    }
}

pub fn simplify_node_in_place(graph: &mut Graph, id: NodeId) -> bool {
    let Some(rewrite) = algebraic_rewrite(graph, id) else {
        return false;
    };
    match rewrite {
        Rewrite::Node(replacement) => {
            if replacement == id {
                return false;
            }
            return graph.replace_uses(id, replacement) != 0;
        }
        Rewrite::Constant(value) => {
            let node = graph.node_mut(id).expect("simplified node exists");
            node.kind = NodeKind::Const(value);
            node.data.clear();
        }
        Rewrite::Comparison {
            operation,
            left,
            right,
        } => {
            let node = graph.node_mut(id).expect("simplified node exists");
            node.kind = NodeKind::Compare(operation);
            node.data = vec![left, right];
        }
    }
    true
}

fn simplify_binary(graph: &Graph, op: BinaryOp, left: NodeId, right: NodeId) -> Option<Rewrite> {
    let left_const = const_value(graph, left);
    let right_const = const_value(graph, right);
    match op {
        BinaryOp::Add => {
            if left_const == Some(0) {
                Some(Rewrite::Node(right))
            } else if right_const == Some(0) {
                Some(Rewrite::Node(left))
            } else {
                None
            }
        }
        BinaryOp::Sub => {
            if right_const == Some(0) {
                Some(Rewrite::Node(left))
            } else if left == right {
                Some(Rewrite::Constant(0))
            } else {
                None
            }
        }
        BinaryOp::Mul => {
            if left_const == Some(0) || right_const == Some(0) {
                Some(Rewrite::Constant(0))
            } else if left_const == Some(1) {
                Some(Rewrite::Node(right))
            } else if right_const == Some(1) {
                Some(Rewrite::Node(left))
            } else {
                None
            }
        }
        BinaryOp::Div | BinaryOp::UDiv => (right_const == Some(1)).then_some(Rewrite::Node(left)),
        BinaryOp::Rem | BinaryOp::URem => (right_const == Some(1)).then_some(Rewrite::Constant(0)),
        BinaryOp::And => {
            if left_const == Some(0) || right_const == Some(0) {
                Some(Rewrite::Constant(0))
            } else if left_const == Some(-1) {
                Some(Rewrite::Node(right))
            } else if right_const == Some(-1) || left == right {
                Some(Rewrite::Node(left))
            } else {
                None
            }
        }
        BinaryOp::Or => {
            if left_const == Some(-1) || right_const == Some(-1) {
                Some(Rewrite::Constant(-1))
            } else if left_const == Some(0) {
                Some(Rewrite::Node(right))
            } else if right_const == Some(0) || left == right {
                Some(Rewrite::Node(left))
            } else {
                None
            }
        }
        BinaryOp::Xor => {
            if left == right {
                Some(Rewrite::Constant(0))
            } else if left_const == Some(0) {
                Some(Rewrite::Node(right))
            } else if right_const == Some(0) {
                Some(Rewrite::Node(left))
            } else {
                None
            }
        }
        BinaryOp::Shl | BinaryOp::Shr | BinaryOp::LShr => {
            if left_const == Some(0) {
                Some(Rewrite::Constant(0))
            } else if right_const == Some(0) {
                Some(Rewrite::Node(left))
            } else {
                None
            }
        }
    }
}

fn is_root(graph: &Graph, _id: NodeId, node: &Node) -> bool {
    match node.kind {
        NodeKind::Return => true,
        NodeKind::Binary(BinaryOp::Div | BinaryOp::Rem | BinaryOp::UDiv | BinaryOp::URem) => {
            division_may_trap(graph, node)
        }
        _ => false,
    }
}

fn division_may_trap(graph: &Graph, node: &Node) -> bool {
    if node.data.len() != 2 {
        return true;
    }
    let Some(left) = const_value(graph, node.data[0]) else {
        return true;
    };
    let Some(right) = const_value(graph, node.data[1]) else {
        return true;
    };
    right == 0
        || (matches!(node.kind, NodeKind::Binary(BinaryOp::Div | BinaryOp::Rem))
            && left == i64::MIN
            && right == -1)
}

fn bool_value(value: bool) -> i64 {
    if value { 1 } else { 0 }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn returned_constant(graph: &Graph) -> Option<i64> {
        let (_, result) = graph
            .iter()
            .find(|(_, node)| node.kind == NodeKind::Return)?;
        const_value(graph, result.data[0])
    }

    #[test]
    fn folds_integer_operations_and_comparisons() {
        let cases = [
            (BinaryOp::Add, i64::MAX, 1, i64::MIN),
            (BinaryOp::Sub, i64::MIN, 1, i64::MAX),
            (BinaryOp::Mul, 7, 6, 42),
            (BinaryOp::Div, -42, 2, -21),
            (BinaryOp::Rem, -43, 2, -1),
            (BinaryOp::And, 0b1100, 0b1010, 0b1000),
            (BinaryOp::Or, 0b1100, 0b1010, 0b1110),
            (BinaryOp::Xor, 0b1100, 0b1010, 0b0110),
            (BinaryOp::Shl, 1, 65, 2),
            (BinaryOp::Shr, -8, 1, -4),
        ];
        for (op, left, right, expected) in cases {
            let mut graph = Graph::new();
            let start = graph.start();
            let left = graph.constant(left);
            let right = graph.constant(right);
            let value = graph.binary(op, left, right);
            graph.return_(start, Some(value));
            optimize(&mut graph);
            assert_eq!(returned_constant(&graph), Some(expected), "{op:?}");
            graph.validate().unwrap();
        }

        let mut graph = Graph::new();
        let start = graph.start();
        let value = graph.constant(i64::MIN);
        let negated = graph.unary(UnaryOp::Neg, value);
        let inverted = graph.unary(UnaryOp::Not, negated);
        let max = graph.constant(i64::MAX);
        let comparison = graph.compare(CompareOp::Eq, inverted, max);
        graph.return_(start, Some(comparison));
        let stats = optimize(&mut graph);
        assert_eq!(returned_constant(&graph), Some(1));
        assert_eq!(stats.constants_folded, 3);
    }

    #[test]
    fn trapping_division_is_not_folded_or_discarded() {
        for (left, right) in [(1, 0), (i64::MIN, -1)] {
            let mut graph = Graph::new();
            let left = graph.constant(left);
            let right = graph.constant(right);
            let division = graph.binary(BinaryOp::Div, left, right);
            let stats = optimize(&mut graph);

            assert_eq!(stats.constants_folded, 0);
            assert!(graph.node(division).is_some());
            assert_eq!(graph.len(), 3);
        }
    }

    #[test]
    fn simplifies_algebra_and_phi_nodes() {
        let mut graph = Graph::new();
        let start = graph.start();
        let other_start = graph.start();
        let parameter = graph.parameter(0);
        let zero = graph.constant(0);
        let all_ones = graph.constant(-1);
        let and = graph.binary(BinaryOp::And, parameter, all_ones);
        let or = graph.binary(BinaryOp::Or, and, zero);
        let difference = graph.binary(BinaryOp::Sub, parameter, parameter);
        let sum = graph.binary(BinaryOp::Add, or, difference);
        let phi = graph.phi(vec![start, other_start], vec![sum, sum]);
        graph.return_(start, Some(phi));

        let stats = optimize(&mut graph);
        assert!(stats.algebraic_simplifications >= 5);
        let (_, result) = graph
            .iter()
            .find(|(_, node)| node.kind == NodeKind::Return)
            .unwrap();
        assert_eq!(result.data, vec![parameter]);
        assert!(graph.node(phi).is_none());
        graph.validate().unwrap();
    }

    #[test]
    fn eliminates_dead_nodes_but_preserves_memory_dependencies() {
        let mut graph = Graph::new();
        let start = graph.start();
        let left = graph.constant(20);
        let right = graph.constant(22);
        let memory_value = graph.binary(BinaryOp::Add, left, right);
        let dead = graph.binary(BinaryOp::Mul, left, right);
        let result = graph.add_node_with_memory(
            NodeKind::Return,
            vec![start],
            Vec::new(),
            vec![memory_value],
        );

        assert_eq!(eliminate_dead_nodes(&mut graph), 1);
        assert!(graph.node(result).is_some());
        assert!(graph.node(memory_value).is_some());
        assert!(graph.node(dead).is_none());
        graph.validate().unwrap();
    }

    #[test]
    fn dead_node_elimination_handles_phi_cycles() {
        let mut graph = Graph::new();
        let start = graph.start();
        let initial = graph.parameter(0);
        let one = graph.constant(1);
        let phi = graph.phi(vec![start, start], vec![initial, initial]);
        let next = graph.binary(BinaryOp::Add, phi, one);
        graph.node_mut(phi).unwrap().data[1] = next;
        graph.return_(start, Some(phi));

        assert_eq!(eliminate_dead_nodes(&mut graph), 0);
        graph.validate().unwrap();
    }

    #[test]
    fn optimize_rewrites_a_complete_graph_to_a_fixed_point() {
        let mut graph = Graph::new();
        let start = graph.start();
        let parameter = graph.parameter(0);
        let forty = graph.constant(40);
        let two = graph.constant(2);
        let folded = graph.binary(BinaryOp::Add, forty, two);
        let zero = graph.binary(BinaryOp::Xor, parameter, parameter);
        let result = graph.binary(BinaryOp::Add, folded, zero);
        graph.return_(start, Some(result));
        let dead = graph.binary(BinaryOp::Mul, forty, two);
        graph.node_mut(dead).unwrap().data.clear();

        let stats = optimize(&mut graph);
        assert_eq!(stats.constants_folded, 1);
        assert!(stats.algebraic_simplifications >= 2);
        assert_eq!(returned_constant(&graph), Some(42));
        assert!(graph.node(dead).is_none());
        graph.validate().unwrap();
    }
}
