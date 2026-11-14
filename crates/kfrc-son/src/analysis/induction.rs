//! Exact modular integer recurrences on a two-input loop header. These facts
//! say nothing about termination, trip counts, pointer bounds or no-wrap.
use crate::{FunctionGraph, NodeId, Opcode};
use kfrc_body::{TyId, TypeOracle, TypeShape};

#[derive(Clone, Copy, Debug)]
pub(crate) struct Induction {
    pub initial: NodeId,
    pub ty: TyId,
    pub mask: u64,
    /// Additive step, reduced modulo the induction's own integer width.
    pub step: u64,
}

pub(crate) fn constant(graph: &FunctionGraph, node: NodeId) -> Option<u64> {
    let node = graph.resolve(node);
    (graph.op(node) == Opcode::Const).then(|| graph.constant_value(node) as u64)
}

impl Induction {
    /// The caller proves which header predecessor is the entry and which is
    /// the backedge. Both the induction and its update must be ordinary typed
    /// wrapping integers; NSW/NUW or any other arithmetic promise is rejected.
    pub(crate) fn analyze(graph: &FunctionGraph, phi: NodeId, entry: usize) -> Option<Self> {
        let phi = graph.resolve(phi);
        if entry > 1 || graph.op(phi) != Opcode::Phi || graph.inputs(phi).len() != 3 {
            return None;
        }
        let header = graph.resolve(graph.input(phi, 0));
        if graph.op(header) != Opcode::Region || graph.inputs(header).len() != 2 {
            return None;
        }
        let ty = graph.rust_type(phi)?;
        let TypeShape::Int { width, .. } = graph.oracle.shape(ty) else {
            return None;
        };
        let bits = width.bits(64);
        if bits > 64 {
            return None;
        }
        let mask = u64::MAX >> (64 - bits);
        let update = graph.resolve(graph.input(phi, 2 - entry));
        if !matches!(graph.op(update), Opcode::Add | Opcode::Sub)
            || graph.payload(update) != 0
            || !graph.control(update).is_none()
            || graph.rust_type(update) != Some(ty)
        {
            return None;
        }
        let left = graph.resolve(graph.input(update, 0));
        let right = graph.resolve(graph.input(update, 1));
        let amount = if left == phi {
            right
        } else if right == phi && graph.op(update) == Opcode::Add {
            left
        } else {
            return None;
        };
        if graph.rust_type(amount) != Some(ty) {
            return None;
        }
        let step = constant(graph, amount)? & mask;
        let step = if graph.op(update) == Opcode::Sub {
            0u64.wrapping_sub(step) & mask
        } else {
            step
        };
        Some(Self {
            initial: graph.resolve(graph.input(phi, entry + 1)),
            ty,
            mask,
            step,
        })
    }
}
