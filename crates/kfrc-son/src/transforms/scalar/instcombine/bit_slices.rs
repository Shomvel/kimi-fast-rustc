//! Reassemble unsigned slices of one integer without a byte packing chain.
//! Each accepted term still depends on the same source, including its poison.
use super::*;
use crate::ValueType;

/// Recognize x & mask, or ((x >> k) as uN as word) << k. Both denote
/// exactly x & mask. Signed narrow extensions and promised shifts stay intact.
fn slice(graph: &FunctionGraph, node: NodeId, ty: TyId, word: Integer) -> Option<(NodeId, u64)> {
    let node = graph.resolve(node);
    if graph.rust_type(node) != Some(ty)
        || graph.payload(node) != 0
        || !graph.control(node).is_none()
    {
        return None;
    }
    if graph.op(node) == Opcode::BitAnd {
        for (source, mask) in [(0, 1), (1, 0)] {
            if let Some(mask) = constant(graph, graph.input(node, mask)) {
                return Some((graph.resolve(graph.input(node, source)), mask));
            }
        }
    }
    let (value, left) = if graph.op(node) == Opcode::ShiftLeft {
        (
            graph.resolve(graph.input(node, 0)),
            constant(graph, graph.input(node, 1))?,
        )
    } else {
        (node, 0)
    };
    if left >= u64::from(word.bits) {
        return None;
    }
    let (value, mask) = if graph.op(value) == Opcode::BitAnd {
        if graph.payload(value) != 0 || !graph.control(value).is_none() {
            return None;
        }
        let (source, mask) = if let Some(mask) = constant(graph, graph.input(value, 1)) {
            (graph.input(value, 0), mask)
        } else {
            (
                graph.input(value, 1),
                constant(graph, graph.input(value, 0))?,
            )
        };
        // Unsigned truncation/extension canonicalizes to this same mask.
        (graph.resolve(source), mask)
    } else if graph.op(value) == Opcode::IntCast {
        if graph.payload(value) != 0 || !graph.control(value).is_none() {
            return None;
        }
        let narrow = graph.resolve(graph.input(value, 0));
        let narrow_ty = graph.rust_type(narrow)?;
        if !matches!(
            graph.oracle.shape(narrow_ty),
            TypeShape::Int { signed: false, .. }
        ) {
            return None;
        }
        let narrow_type = integer(graph, narrow_ty)?;
        if narrow_type.signed
            || narrow_type.bits >= word.bits
            || graph.op(narrow) != Opcode::IntCast
            || graph.payload(narrow) != 0
            || !graph.control(narrow).is_none()
        {
            return None;
        }
        (
            graph.resolve(graph.input(narrow, 0)),
            u64::MAX >> (64 - narrow_type.bits),
        )
    } else {
        (value, word.mask())
    };
    if graph.rust_type(value) != Some(ty) {
        return None;
    }
    let (source, right) = if graph.op(value) == Opcode::ShiftRightLogical {
        if graph.payload(value) != 0 || !graph.control(value).is_none() {
            return None;
        }
        (
            graph.resolve(graph.input(value, 0)),
            constant(graph, graph.input(value, 1))?,
        )
    } else {
        (value, 0)
    };
    if left != right {
        return None;
    }
    Some((source, (mask << left) & word.mask()))
}

pub(super) fn combine(
    graph: &mut FunctionGraph,
    node: NodeId,
    ty: TyId,
    word: Integer,
) -> Option<NodeId> {
    if graph.op(node) != Opcode::BitOr
        || graph.payload(node) != 0
        || !graph.control(node).is_none()
        || !matches!(graph.oracle.shape(ty), TypeShape::Int { .. })
    {
        return None;
    }
    let (left, a) = slice(graph, graph.input(node, 0), ty, word)?;
    let (right, b) = slice(graph, graph.input(node, 1), ty, word)?;
    if left != right {
        return None;
    }
    let mask = a | b;
    if mask == word.mask() {
        Some(left)
    } else {
        let mask = graph.add_constant(u128::from(mask), ty);
        Some(graph.add_node(
            Opcode::BitAnd,
            NodeId::NONE,
            &[left, mask],
            0,
            ValueType::Rust(ty),
        ))
    }
}
