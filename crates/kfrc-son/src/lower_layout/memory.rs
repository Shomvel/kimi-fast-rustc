//! Physical field offsets and ABI snapshots are introduced only at this seam.

use super::*;
use kfrc_body::FieldLayout;

impl LayoutLowering<'_> {
    pub(super) fn fields(
        &self,
        ty: TyId,
        variant: Option<u32>,
    ) -> Result<Vec<FieldLayout>, LayoutError> {
        if let TypeShape::Array { element, length } = self.graph.oracle.shape(ty) {
            let layout = self
                .graph
                .oracle
                .size_align(*element)
                .ok_or_else(|| LayoutError::Unsupported("unsized array element".into()))?;
            return (0..*length)
                .map(|index| {
                    Ok(FieldLayout {
                        ty: *element,
                        offset: index.checked_mul(layout.size).ok_or_else(|| {
                            LayoutError::Malformed("array offset overflow".into())
                        })?,
                    })
                })
                .collect();
        }
        let aggregate = self.graph.oracle.aggregates.get(&ty).ok_or_else(|| {
            LayoutError::Unsupported(format!(
                "missing field layout for {}",
                self.graph.oracle.describe(ty)
            ))
        })?;
        match variant {
            None => Ok(aggregate.fields.clone()),
            Some(index) => aggregate
                .variants
                .get(index as usize)
                .cloned()
                .ok_or_else(|| LayoutError::Malformed("variant layout index out of bounds".into())),
        }
    }

    pub(super) fn field_address(&mut self, base: NodeId, offset: u64, ty: TyId) -> NodeId {
        let index_ty = self.usize_type();
        let offset = self.graph.add_constant(u128::from(offset), index_ty);
        let pointer = self.pointer_type(ty);
        self.graph.add_node(
            Opcode::PtrOffset,
            NodeId::NONE,
            &[base, offset],
            0,
            ValueType::Rust(pointer),
        )
    }

    pub(super) fn load(
        &mut self,
        address: NodeId,
        ty: TyId,
        control: &mut NodeId,
        effect: &mut NodeId,
    ) -> NodeId {
        let load = self.graph.add_node(
            Opcode::Load,
            *control,
            &[*effect, address],
            self.partition,
            ValueType::Tuple(vec![
                ValueType::Control,
                ValueType::Effect,
                ValueType::Rust(ty),
            ]),
        );
        *control = self.projection(load, 0, ValueType::Control);
        *effect = self.projection(load, 1, ValueType::Effect);
        self.stats.field_loads += 1;
        self.projection(load, 2, ValueType::Rust(ty))
    }

    pub(super) fn store(
        &mut self,
        address: NodeId,
        value: NodeId,
        control: &mut NodeId,
        effect: &mut NodeId,
    ) {
        let store = self.graph.add_node(
            Opcode::Store,
            *control,
            &[*effect, address, value],
            self.partition,
            ValueType::Tuple(vec![ValueType::Control, ValueType::Effect]),
        );
        *control = self.projection(store, 0, ValueType::Control);
        *effect = self.projection(store, 1, ValueType::Effect);
        self.stats.field_stores += 1;
    }

    pub(super) fn decode_fields(
        &mut self,
        address: NodeId,
        ty: TyId,
        control: &mut NodeId,
        effect: &mut NodeId,
    ) -> Result<Value, LayoutError> {
        if self
            .graph
            .oracle
            .semantic()
            .maybe_uninit_inner(ty)
            .is_some()
        {
            return self.decode_opaque(address, ty, control, effect);
        }
        if self.graph.oracle.semantic().is_enum(ty) {
            return self.decode_variant(address, ty, control, effect);
        }
        let parts = if self.composite(ty) {
            let mut values = Vec::new();
            for field in self.fields(ty, None)? {
                let address = self.field_address(address, field.offset, field.ty);
                values.push(self.decode_fields(address, field.ty, control, effect)?);
            }
            Parts::Fields(values)
        } else {
            let layout = self
                .graph
                .oracle
                .size_align(ty)
                .ok_or_else(|| LayoutError::Unsupported("unsized ABI field".into()))?;
            let scalar = if layout.size == 0 {
                self.graph.add_constant(0, ty)
            } else {
                self.load(address, ty, control, effect)
            };
            Parts::Scalar(scalar)
        };
        Ok(Value { ty, parts })
    }

    pub(super) fn decode_boundary(&mut self, node: NodeId, ty: TyId) -> Result<Value, LayoutError> {
        let source = self.graph.input(node, 0);
        if !matches!(
            self.graph.op(source),
            Opcode::Start | Opcode::Call | Opcode::CallIndirect | Opcode::Opaque
        ) {
            return Err(LayoutError::Unsupported(
                "aggregate projection outside an ABI boundary".into(),
            ));
        }
        self.graph.set_value_type(node, ValueType::LayoutValue(ty));
        let before: Vec<_> = self.graph.ids().collect();
        let old_control = self.projection(source, 0, ValueType::Control);
        let old_effect = self.projection(source, 1, ValueType::Effect);
        // Earlier parameter snapshots may already follow the start projection.
        // Insert this snapshot immediately after the source, before those reads.
        let mut control = old_control;
        let mut effect = old_effect;
        let value = if self.encoded_values.contains(&node) {
            self.decode_encoded(node, ty, &mut control, &mut effect)?
        } else {
            self.decode_fields(node, ty, &mut control, &mut effect)?
        };
        for user in before {
            if self.graph.control(user) == old_control {
                self.graph.set_control(user, control);
            }
            let inputs = self.graph.inputs(user).to_vec();
            for (index, input) in inputs.into_iter().enumerate() {
                if input == old_effect {
                    self.graph.set_input(user, index, effect);
                }
            }
        }
        self.values.insert(node, value.clone());
        Ok(value)
    }

    pub(super) fn encode_fields(
        &mut self,
        value: &Value,
        address: NodeId,
        control: &mut NodeId,
        effect: &mut NodeId,
    ) -> Result<(), LayoutError> {
        match &value.parts {
            Parts::OpaqueStorage(source) => {
                let size = self
                    .graph
                    .oracle
                    .size_align(value.ty)
                    .ok_or_else(|| LayoutError::Unsupported("unsized MaybeUninit".into()))?
                    .size;
                self.raw_copy(*source, address, size, control, effect);
            }
            Parts::Scalar(node) => {
                let layout = self
                    .graph
                    .oracle
                    .size_align(value.ty)
                    .ok_or_else(|| LayoutError::Unsupported("unsized value field".into()))?;
                if layout.size != 0 {
                    self.store(address, *node, control, effect);
                }
            }
            Parts::Fields(values) => {
                let fields = self.fields(value.ty, None)?;
                if values.len() != fields.len() {
                    return Err(LayoutError::Malformed(
                        "value/layout field count mismatch".into(),
                    ));
                }
                for (value, field) in values.iter().zip(fields) {
                    if value.ty != field.ty {
                        return Err(LayoutError::Malformed(
                            "value/layout field type mismatch".into(),
                        ));
                    }
                    let address = self.field_address(address, field.offset, field.ty);
                    self.encode_fields(value, address, control, effect)?;
                }
            }
            Parts::Variant { .. } | Parts::KnownVariant { .. } => {
                self.encode_variant(value, address, control, effect)?
            }
            Parts::Repeat {
                element,
                count,
                overrides,
            } => {
                self.encode_repeat(element, *count, overrides, address, control, effect)?;
            }
            Parts::VariantTag(_) | Parts::EncodedVariant(_) => {
                return Err(LayoutError::Malformed(
                    "tag-only enum escaped its discriminant consumers".into(),
                ));
            }
        }
        Ok(())
    }

    pub(super) fn materialize_before(
        &mut self,
        value: &Value,
        consumer: NodeId,
    ) -> Result<NodeId, LayoutError> {
        let layout = self
            .graph
            .oracle
            .size_align(value.ty)
            .ok_or_else(|| LayoutError::Unsupported("unsized ABI value".into()))?;
        let slot = self.graph.stack_slots.len() as u32;
        self.graph.stack_slots.push(layout);
        self.stats.abi_slots += 1;
        let pointer = self.pointer_type(value.ty);
        let address = self.graph.add_node(
            Opcode::StackSlot,
            NodeId::NONE,
            &[],
            slot,
            ValueType::Rust(pointer),
        );
        let mut control = self.graph.control(consumer);
        let mut effect = self.graph.input(consumer, 0);
        self.encode_fields(value, address, &mut control, &mut effect)?;
        self.graph.set_control(consumer, control);
        self.graph.set_input(consumer, 0, effect);
        let index_ty = self.usize_type();
        let zero = self.graph.add_constant(0, index_ty);
        Ok(self.graph.add_node(
            Opcode::PtrOffset,
            NodeId::NONE,
            &[address, zero],
            0,
            ValueType::LayoutValue(value.ty),
        ))
    }
}
