use crate::{FunctionGraph, NodeId, Opcode, ValueType};
use kfrc_body::{TypeOracle, TypeShape};

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(super) struct Expression {
    opcode: Opcode,
    ty: ValueType,
    payload: u32,
    inputs: Vec<NodeId>,
}

impl Expression {
    pub fn of(graph: &FunctionGraph, source: NodeId, inputs: &[NodeId]) -> Self {
        let mut inputs: Vec<_> = inputs.iter().map(|&node| graph.resolve(node)).collect();
        if matches!(
            graph.op(source),
            Opcode::Add | Opcode::Mul | Opcode::BitAnd | Opcode::BitOr | Opcode::BitXor
        ) {
            inputs.sort_unstable();
        }
        Self {
            opcode: graph.op(source),
            ty: graph.value_type(source).clone(),
            payload: graph.payload(source),
            inputs,
        }
    }
}

pub(super) fn scalar(graph: &FunctionGraph, node: NodeId) -> bool {
    graph
        .rust_type(node)
        .is_some_and(|ty| match graph.oracle.shape(ty) {
            TypeShape::Bool | TypeShape::Char => true,
            TypeShape::Int { width, .. } => width.bits(64) <= 64,
            _ => false,
        })
}

/// Every listed operation is strict in its scalar inputs and has no eager UB.
/// This first implementation leaves integer intrinsics for a separate pass.
pub(super) fn scalar_op(op: Opcode) -> bool {
    matches!(
        op,
        Opcode::Add
            | Opcode::Sub
            | Opcode::Mul
            | Opcode::BitAnd
            | Opcode::BitOr
            | Opcode::BitXor
            | Opcode::ShiftLeft
            | Opcode::ShiftRightArithmetic
            | Opcode::ShiftRightLogical
            | Opcode::Negate
            | Opcode::BitNot
            | Opcode::Compare
            | Opcode::IntCast
    )
}

pub(super) fn eligible(graph: &FunctionGraph, node: NodeId) -> bool {
    scalar_op(graph.op(node))
        && scalar(graph, node)
        && graph.control(node).is_none()
        && (graph.op(node) == Opcode::Compare || graph.payload(node) == 0)
        && graph
            .inputs(node)
            .iter()
            .all(|&input| scalar(graph, graph.resolve(input)))
}
