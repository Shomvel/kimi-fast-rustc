//! Operations that cannot introduce poison or fail when their scalar inputs
//! are defined. This says nothing about flags, memory access or representation
//! decoding; the observation proof must still prove every operand at its use.
use crate::{FunctionGraph, NodeId, Opcode};
use kfrc_body::{TypeOracle, TypeShape};

pub(super) fn propagates_defined_inputs(graph: &FunctionGraph, node: NodeId) -> bool {
    if !graph.control(node).is_none() || !scalar(graph, node) {
        return false;
    }
    let inputs = match graph.op(node) {
        Opcode::Compare => 2,
        Opcode::Add
        | Opcode::Sub
        | Opcode::Mul
        | Opcode::MulOverflow
        | Opcode::BitAnd
        | Opcode::BitOr
        | Opcode::BitXor
        | Opcode::ShiftLeft
        | Opcode::ShiftRightLogical
        | Opcode::ShiftRightArithmetic
            if graph.payload(node) == 0 =>
        {
            2
        }
        Opcode::Negate | Opcode::BitNot | Opcode::IntCast if graph.payload(node) == 0 => 1,
        _ => return false,
    };
    graph.inputs(node).len() == inputs
        && graph.inputs(node).iter().all(|&input| scalar(graph, input))
}

fn scalar(graph: &FunctionGraph, node: NodeId) -> bool {
    graph
        .rust_type(node)
        .is_some_and(|ty| match graph.oracle.shape(ty) {
            TypeShape::Int { width, .. } => width.bits(64) <= 64,
            TypeShape::Bool | TypeShape::Char => true,
            _ => false,
        })
}
