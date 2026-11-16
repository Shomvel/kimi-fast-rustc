//! Whole values crossing ABI boundaries keep their representation. Transport
//! does not decode every variant, validate padding, or reconstruct fields.
//! A source whose consumers inspect fields stays on the field-demand path.

use super::*;
use crate::analysis::uses::{UseKind, Uses};

pub(super) fn select(graph: &FunctionGraph) -> HashSet<NodeId> {
    let uses = Uses::new(graph);
    graph
        .ids()
        .filter(|&node| {
            if graph.op(node) != Opcode::Projection
                || !matches!(
                    (graph.op(graph.input(node, 0)), graph.payload(node)),
                    (Opcode::Start, 2..) | (Opcode::Call | Opcode::CallIndirect, 2)
                )
            {
                return false;
            }
            let Some(ty) = graph.rust_type(node) else {
                return false;
            };
            if !graph.oracle.semantic().is_product(ty)
                && !graph.oracle.semantic().is_enum(ty)
                && !matches!(graph.oracle.shape(ty), TypeShape::Array { .. })
                && graph.oracle.semantic().maybe_uninit_inner(ty).is_none()
            {
                return false;
            }
            !uses.of(node).is_empty()
                && uses.of(node).iter().all(|use_| {
                    matches!(
                        (graph.op(use_.user), use_.kind()),
                        (Opcode::Return, UseKind::Operand(1))
                            | (Opcode::StoreValue, UseKind::Operand(2))
                            | (Opcode::Call, UseKind::Operand(1..))
                            | (Opcode::CallIndirect, UseKind::Operand(2..))
                    )
                })
        })
        .collect()
}

impl LayoutLowering<'_> {
    pub(super) fn transport_boundary(
        &mut self,
        node: NodeId,
        ty: TyId,
        control: &mut NodeId,
        effect: &mut NodeId,
    ) -> Result<Value, LayoutError> {
        self.stats.whole_value_transports += 1;
        // A call result already belongs to this invocation. Parameters may
        // point at caller-owned storage: capture their bytes at entry before
        // any call can mutate memory reachable through another argument.
        let source = self.graph.input(node, 0);
        let address = if self.graph.op(source) == Opcode::Start {
            let destination = self.opaque_slot(ty)?;
            let size = self
                .graph
                .oracle
                .size_align(ty)
                .ok_or_else(|| LayoutError::Unsupported("unsized ABI transport".into()))?
                .size;
            self.raw_copy(node, destination, size, control, effect);
            destination
        } else {
            node
        };
        Ok(Value {
            ty,
            parts: Parts::OwnedStorage(address),
        })
    }
}
