//! Demand-based rewrites preserve the variable operand and every poison flag.
//! Whole-value plans use the union of every use. Per-use plans only rebuild
//! their consumer, leaving shared producers intact for their other users.
use super::*;
use crate::analysis::{demanded_bits, uses::Uses};

#[derive(Clone, Copy)]
enum Replacement {
    Value(NodeId),
    LogicalShift(NodeId),
}

fn plan(graph: &FunctionGraph, node: NodeId, demanded: u64) -> Option<Replacement> {
    let node = graph.resolve(node);
    if graph.payload(node) != 0 || !graph.control(node).is_none() {
        return None;
    }
    let ty = graph.rust_type(node)?;
    // These rewrites change unobserved bits. Restrict their result to ordinary
    // integers rather than constructing potentially invalid char/bool values.
    if !matches!(graph.oracle.shape(ty), TypeShape::Int { .. }) {
        return None;
    }
    let scalar = integer(graph, ty)?;
    let demanded = demanded & scalar.mask();
    if matches!(
        graph.op(node),
        Opcode::BitAnd | Opcode::BitOr | Opcode::BitXor
    ) {
        for (value, bits) in [(0, 1), (1, 0)] {
            let value = graph.resolve(graph.input(node, value));
            let Some(bits) = constant(graph, graph.input(node, bits)) else {
                continue;
            };
            let unchanged = if graph.op(node) == Opcode::BitAnd {
                demanded & !bits == 0
            } else {
                demanded & bits == 0
            };
            if unchanged && graph.rust_type(value) == Some(ty) {
                return Some(Replacement::Value(value));
            }
        }
    }
    if graph.op(node) == Opcode::ShiftRightArithmetic {
        let amount = constant(graph, graph.input(node, 1))? % u64::from(scalar.bits);
        let sign_filled = scalar.mask() ^ (scalar.mask() >> amount);
        if demanded & sign_filled == 0 {
            return Some(Replacement::LogicalShift(node));
        }
    }
    None
}

fn materialize(graph: &mut FunctionGraph, replacement: Replacement) -> NodeId {
    match replacement {
        Replacement::Value(value) => graph.resolve(value),
        Replacement::LogicalShift(node) => {
            let inputs = graph.inputs(node).to_vec();
            graph.add_node(
                Opcode::ShiftRightLogical,
                NodeId::NONE,
                &inputs,
                0,
                graph.value_type(node).clone(),
            )
        }
    }
}

/// All plans are proved against one immutable use snapshot, then applied.
/// Removing a mask is allowed only when every demanded bit passes unchanged;
/// therefore the batch cannot introduce any additional input-bit demand.
pub(super) fn merge_uses(graph: &mut FunctionGraph) -> usize {
    let candidates: Vec<_> = graph
        .ids()
        .filter(|&node| {
            graph.payload(node) == 0
                && graph.control(node).is_none()
                && matches!(
                    graph.op(node),
                    Opcode::BitAnd | Opcode::BitOr | Opcode::BitXor | Opcode::ShiftRightArithmetic
                )
        })
        .collect();
    if candidates.is_empty() {
        return 0;
    }
    let uses = Uses::new(graph);
    let plans: Vec<_> = candidates
        .into_iter()
        .filter_map(|node| {
            let demanded = demanded_bits::of(graph, &uses, node)?;
            Some((node, plan(graph, node, demanded)?))
        })
        .collect();
    let count = plans.len();
    for (node, replacement) in plans {
        let value = materialize(graph, replacement);
        graph.replace(node, value);
    }
    count
}

/// The shared scalar entry point also handles a single consumer, including
/// splice seams. It never globally forwards a shared value under one use's
/// demand, and never clones the producer's surrounding expression tree.
pub(super) fn combine(graph: &mut FunctionGraph, node: NodeId) -> Option<NodeId> {
    if !matches!(
        graph.op(node),
        Opcode::IntCast
            | Opcode::BitAnd
            | Opcode::BitOr
            | Opcode::BitXor
            | Opcode::BitNot
            | Opcode::ShiftLeft
            | Opcode::ShiftRightLogical
            | Opcode::ShiftRightArithmetic
    ) || graph.payload(node) != 0
        || !graph.control(node).is_none()
    {
        return None;
    }
    let (bits, _) = demanded_bits::width(graph, node)?;
    for slot in 0..graph.inputs(node).len() {
        let Some(demanded) = demanded_bits::operand(graph, node, slot, demanded_bits::mask(bits))
        else {
            continue;
        };
        let input = graph.input(node, slot);
        if let Some(replacement) = plan(graph, input, demanded) {
            let mut inputs = graph.inputs(node).to_vec();
            inputs[slot] = materialize(graph, replacement);
            return Some(graph.add_node(
                graph.op(node),
                graph.control(node),
                &inputs,
                graph.payload(node),
                graph.value_type(node).clone(),
            ));
        }
    }
    None
}
