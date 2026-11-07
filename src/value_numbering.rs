use std::collections::{BTreeMap, BTreeSet};

use crate::son::{BinaryOp, CompareOp, Graph, NodeId, NodeKind, UnaryOp};

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum ExpressionKey {
    Const(i64),
    Parameter(usize),
    Unary(u8, NodeId),
    Binary(u8, NodeId, NodeId),
    Compare(u8, NodeId, NodeId),
}

pub fn value_numbering(graph: &mut Graph) -> usize {
    let mut merged_ids = BTreeSet::new();
    let mut merged = 0;
    loop {
        let pass_merged = value_numbering_pass(graph, &mut merged_ids);
        merged += pass_merged;
        if pass_merged == 0 {
            return merged;
        }
    }
}

fn value_numbering_pass(graph: &mut Graph, merged_ids: &mut BTreeSet<NodeId>) -> usize {
    let ids: Vec<_> = graph.ids().collect();
    let mut expressions: BTreeMap<u64, Vec<(ExpressionKey, NodeId)>> = BTreeMap::new();
    let mut merged = 0;

    for id in ids {
        if merged_ids.contains(&id) {
            continue;
        }
        let Some(key) = expression_key(graph, id) else {
            continue;
        };
        let hash = structural_hash(&key);
        let bucket = expressions.entry(hash).or_default();
        let representative = bucket
            .iter()
            .find_map(|(candidate, representative)| (candidate == &key).then_some(*representative));

        if let Some(representative) = representative {
            graph.replace_uses(id, representative);
            merged_ids.insert(id);
            merged += 1;
        } else {
            bucket.push((key, id));
        }
    }

    merged
}

fn expression_key(graph: &Graph, id: NodeId) -> Option<ExpressionKey> {
    let node = graph.node(id)?;
    if !node.control.is_empty() || !node.memory.is_empty() {
        return None;
    }

    match node.kind {
        NodeKind::Const(value) if node.data.is_empty() => Some(ExpressionKey::Const(value)),
        NodeKind::Parameter { index } if node.data.is_empty() => {
            Some(ExpressionKey::Parameter(index))
        }
        NodeKind::Unary(op) if node.data.len() == 1 => {
            Some(ExpressionKey::Unary(unary_code(op), node.data[0]))
        }
        NodeKind::Binary(op) if node.data.len() == 2 => {
            let (left, right) = ordered_operands(op, node.data[0], node.data[1]);
            Some(ExpressionKey::Binary(binary_code(op), left, right))
        }
        NodeKind::Compare(op) if node.data.len() == 2 => {
            let (left, right) = if is_commutative_compare(op) && node.data[1] < node.data[0] {
                (node.data[1], node.data[0])
            } else {
                (node.data[0], node.data[1])
            };
            Some(ExpressionKey::Compare(compare_code(op), left, right))
        }
        _ => None,
    }
}

fn ordered_operands(op: BinaryOp, left: NodeId, right: NodeId) -> (NodeId, NodeId) {
    if is_commutative_binary(op) && right < left {
        (right, left)
    } else {
        (left, right)
    }
}

fn is_commutative_binary(op: BinaryOp) -> bool {
    matches!(
        op,
        BinaryOp::Add | BinaryOp::Mul | BinaryOp::And | BinaryOp::Or | BinaryOp::Xor
    )
}

fn is_commutative_compare(op: CompareOp) -> bool {
    matches!(op, CompareOp::Eq | CompareOp::Ne)
}

fn unary_code(op: UnaryOp) -> u8 {
    match op {
        UnaryOp::Neg => 0,
        UnaryOp::Not => 1,
    }
}

fn binary_code(op: BinaryOp) -> u8 {
    match op {
        BinaryOp::Add => 0,
        BinaryOp::Sub => 1,
        BinaryOp::Mul => 2,
        BinaryOp::Div => 3,
        BinaryOp::Rem => 4,
        BinaryOp::And => 5,
        BinaryOp::Or => 6,
        BinaryOp::Xor => 7,
        BinaryOp::Shl => 8,
        BinaryOp::Shr => 9,
        BinaryOp::UDiv => 10,
        BinaryOp::URem => 11,
        BinaryOp::LShr => 12,
    }
}

fn compare_code(op: CompareOp) -> u8 {
    match op {
        CompareOp::Eq => 0,
        CompareOp::Ne => 1,
        CompareOp::Lt => 2,
        CompareOp::Le => 3,
        CompareOp::Gt => 4,
        CompareOp::Ge => 5,
        CompareOp::ULt => 6,
        CompareOp::ULe => 7,
        CompareOp::UGt => 8,
        CompareOp::UGe => 9,
    }
}

fn structural_hash(key: &ExpressionKey) -> u64 {
    const OFFSET_BASIS: u64 = 0xcbf29ce484222325;
    const PRIME: u64 = 0x100000001b3;

    fn write_bytes(hash: &mut u64, bytes: &[u8]) {
        for &byte in bytes {
            *hash ^= u64::from(byte);
            *hash = hash.wrapping_mul(PRIME);
        }
    }

    fn write_u64(hash: &mut u64, value: u64) {
        write_bytes(hash, &value.to_le_bytes());
    }

    fn write_node(hash: &mut u64, id: NodeId) {
        write_bytes(hash, &id.0.to_le_bytes());
    }

    let mut hash = OFFSET_BASIS;
    match key {
        ExpressionKey::Const(value) => {
            write_bytes(&mut hash, &[1]);
            write_bytes(&mut hash, &value.to_le_bytes());
        }
        ExpressionKey::Parameter(index) => {
            write_bytes(&mut hash, &[2]);
            write_u64(&mut hash, *index as u64);
        }
        ExpressionKey::Unary(op, value) => {
            write_bytes(&mut hash, &[3, *op]);
            write_node(&mut hash, *value);
        }
        ExpressionKey::Binary(op, left, right) => {
            write_bytes(&mut hash, &[4, *op]);
            write_node(&mut hash, *left);
            write_node(&mut hash, *right);
        }
        ExpressionKey::Compare(op, left, right) => {
            write_bytes(&mut hash, &[5, *op]);
            write_node(&mut hash, *left);
            write_node(&mut hash, *right);
        }
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merges_duplicate_expressions_without_changing_node_ids() {
        let mut graph = Graph::new();
        let start = graph.start();
        let parameter = graph.parameter(0);
        let duplicate_parameter = graph.parameter(0);
        let constant = graph.constant(7);
        let duplicate_constant = graph.constant(7);
        let negated = graph.unary(UnaryOp::Neg, parameter);
        let duplicate_negated = graph.unary(UnaryOp::Neg, duplicate_parameter);
        let difference = graph.binary(BinaryOp::Sub, negated, constant);
        let duplicate_difference =
            graph.binary(BinaryOp::Sub, duplicate_negated, duplicate_constant);
        let comparison = graph.compare(CompareOp::Lt, difference, constant);
        let duplicate_comparison =
            graph.compare(CompareOp::Lt, duplicate_difference, duplicate_constant);
        let result = graph.return_(start, Some(duplicate_comparison));
        let ids_before: Vec<_> = graph.ids().collect();

        assert_eq!(value_numbering(&mut graph), 5);
        assert_eq!(graph.node(result).unwrap().data, vec![comparison]);
        assert_eq!(
            graph.node(difference).unwrap().data,
            vec![negated, constant]
        );
        assert_eq!(graph.ids().collect::<Vec<_>>(), ids_before);
        assert!(graph.node(duplicate_difference).is_some());
        graph.validate().unwrap();
    }

    #[test]
    fn canonicalizes_commutative_operands_deterministically() {
        for op in [
            BinaryOp::Add,
            BinaryOp::Mul,
            BinaryOp::And,
            BinaryOp::Or,
            BinaryOp::Xor,
        ] {
            let mut graph = Graph::new();
            let start = graph.start();
            let left = graph.parameter(0);
            let right = graph.parameter(1);
            let forward = graph.binary(op, left, right);
            let reversed = graph.binary(op, right, left);
            let result = graph.return_(start, Some(reversed));

            assert_eq!(value_numbering(&mut graph), 1, "{op:?}");
            assert_eq!(graph.node(result).unwrap().data, vec![forward]);
            assert_eq!(graph.node(reversed).unwrap().data, vec![right, left]);
            graph.validate().unwrap();
        }

        for op in [CompareOp::Eq, CompareOp::Ne] {
            let mut graph = Graph::new();
            let start = graph.start();
            let left = graph.parameter(0);
            let right = graph.parameter(1);
            let forward = graph.compare(op, left, right);
            let reversed = graph.compare(op, right, left);
            let result = graph.return_(start, Some(reversed));

            assert_eq!(value_numbering(&mut graph), 1, "{op:?}");
            assert_eq!(graph.node(result).unwrap().data, vec![forward]);
            graph.validate().unwrap();
        }

        let mut graph = Graph::new();
        let left = graph.parameter(0);
        let right = graph.parameter(1);
        let difference = graph.binary(BinaryOp::Sub, left, right);
        let reversed = graph.binary(BinaryOp::Sub, right, left);
        assert_eq!(value_numbering(&mut graph), 0);
        assert_ne!(difference, reversed);
    }

    #[test]
    fn preserves_trapping_division_and_remainder_nodes() {
        for op in [BinaryOp::Div, BinaryOp::Rem] {
            for (left, right) in [(1, 0), (i64::MIN, -1)] {
                let mut graph = Graph::new();
                let start = graph.start();
                let left = graph.constant(left);
                let right = graph.constant(right);
                let first = graph.binary(op, left, right);
                let duplicate = graph.binary(op, left, right);
                let result = graph.return_(start, Some(duplicate));
                let ids_before: Vec<_> = graph.ids().collect();

                assert_eq!(value_numbering(&mut graph), 1);
                assert_eq!(graph.node(result).unwrap().data, vec![first]);
                assert!(graph.node(first).is_some());
                assert!(graph.node(duplicate).is_some());
                assert_eq!(graph.node(duplicate).unwrap().data, vec![left, right]);
                assert_eq!(graph.ids().collect::<Vec<_>>(), ids_before);
                graph.validate().unwrap();
            }
        }
    }

    #[test]
    fn skips_control_and_memory_nodes_but_rewrites_memory_uses() {
        let mut graph = Graph::new();
        let start = graph.start();
        let left = graph.parameter(0);
        let right = graph.parameter(1);
        let pure = graph.binary(BinaryOp::Add, left, right);
        let duplicate = graph.binary(BinaryOp::Add, left, right);
        let with_control = graph.add_node(
            NodeKind::Binary(BinaryOp::Add),
            vec![start],
            vec![left, right],
        );
        let duplicate_with_control = graph.add_node(
            NodeKind::Binary(BinaryOp::Add),
            vec![start],
            vec![left, right],
        );
        let with_memory = graph.add_node_with_memory(
            NodeKind::Binary(BinaryOp::Add),
            Vec::new(),
            vec![left, right],
            vec![start],
        );
        let duplicate_with_memory = graph.add_node_with_memory(
            NodeKind::Binary(BinaryOp::Add),
            Vec::new(),
            vec![left, right],
            vec![start],
        );
        let result = graph.add_node_with_memory(
            NodeKind::Return,
            vec![start],
            vec![duplicate],
            vec![duplicate],
        );

        assert_eq!(value_numbering(&mut graph), 1);
        assert_eq!(graph.node(result).unwrap().data, vec![pure]);
        assert_eq!(graph.node(result).unwrap().memory, vec![pure]);
        assert_eq!(graph.node(with_memory).unwrap().memory, vec![start]);
        for id in [
            with_control,
            duplicate_with_control,
            with_memory,
            duplicate_with_memory,
        ] {
            assert!(graph.node(id).is_some());
        }
    }

    #[test]
    fn hashing_is_stable_and_commutative_normalization_is_internal() {
        let left = NodeId(20);
        let right = NodeId(3);
        let forward = ExpressionKey::Binary(binary_code(BinaryOp::Add), left, right);
        let reversed = ExpressionKey::Binary(binary_code(BinaryOp::Add), right, left);
        let canonical = ExpressionKey::Binary(binary_code(BinaryOp::Add), right, left);

        assert_eq!(structural_hash(&forward), structural_hash(&forward));
        assert_eq!(structural_hash(&reversed), structural_hash(&canonical));
        assert_ne!(forward, reversed);
    }
}
