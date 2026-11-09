//! MaybeUninit is a byte container, never a product or an alias for its T.
use super::*;

impl Builder<'_> {
    pub(in super::super) fn semantic_transmute(
        &mut self,
        value: NodeId,
        target: TyId,
        block: usize,
        control: &mut NodeId,
    ) -> NodeId {
        let effect = self.read_variable(self.effect, block);
        let cast = self.graph.add_node(
            Opcode::TransmuteValue,
            *control,
            &[effect, value],
            0,
            ValueType::Tuple(vec![
                ValueType::Control,
                ValueType::Effect,
                ValueType::Rust(target),
            ]),
        );
        *control = self.projection(cast, 0, ValueType::Control);
        let effect = self.projection(cast, 1, ValueType::Effect);
        self.write_variable(self.effect, block, effect);
        self.projection(cast, 2, ValueType::Rust(target))
    }

    pub(in super::super) fn maybe_uninit_pack(
        &mut self,
        value: NodeId,
        wrapper: TyId,
        field: TyId,
        block: usize,
        control: &mut NodeId,
    ) -> Result<NodeId, BuildError> {
        if matches!(self.graph.oracle.shape(field), TypeShape::Unit) {
            // The uninit field initializes no T bytes. Fixed evaluation of
            // its operand has already happened before this container is made.
            Ok(self.graph.add_node(
                Opcode::Undefined,
                NodeId::NONE,
                &[],
                0,
                ValueType::Rust(wrapper),
            ))
        } else {
            Ok(self.semantic_transmute(value, wrapper, block, control))
        }
    }

    pub(in super::super) fn maybe_uninit_field(
        &mut self,
        value: NodeId,
        field: TyId,
        block: usize,
        control: &mut NodeId,
    ) -> Result<NodeId, BuildError> {
        if matches!(self.graph.oracle.shape(field), TypeShape::Unit) {
            self.semantic_empty_value(field)
        } else {
            // Extracting value is a typed decoding boundary. Invalid bool,
            // enum tags, or uninitialized payloads cannot masquerade as T.
            Ok(self.semantic_transmute(value, field, block, control))
        }
    }
}
