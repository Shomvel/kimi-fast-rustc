//! Compact repeated values and bounded-size materialization.
use super::*;
use crate::ComparePredicate;

impl LayoutLowering<'_> {
    pub(super) fn value_field(&self, value: &Value, index: u32) -> Result<Value, LayoutError> {
        match &value.parts {
            Parts::Fields(fields) => fields.get(index as usize).cloned(),
            Parts::Repeat {
                element,
                count,
                overrides,
            } if u64::from(index) < *count => {
                Some(overrides.get(&index).unwrap_or(element).clone())
            }
            _ => None,
        }
        .ok_or_else(|| LayoutError::Malformed("field index out of bounds".into()))
    }

    pub(super) fn insert_value_field(
        &self,
        mut base: Value,
        index: u32,
        replacement: Value,
    ) -> Result<Value, LayoutError> {
        if matches!(base.parts, Parts::Omitted) {
            let fields = self
                .graph
                .oracle
                .semantic()
                .field_types(base.ty, None)
                .ok_or_else(|| {
                    LayoutError::Malformed("omitted InsertField base lacks fields".into())
                })?;
            base.parts = Parts::Fields(fields.into_iter().map(|ty| self.omitted(ty)).collect());
        }
        match &mut base.parts {
            Parts::Fields(fields) => {
                let field = fields.get_mut(index as usize).ok_or_else(|| {
                    LayoutError::Malformed("InsertField index out of bounds".into())
                })?;
                *field = replacement;
            }
            Parts::Repeat {
                count, overrides, ..
            } if u64::from(index) < *count => {
                overrides.insert(index, replacement);
            }
            _ => {
                return Err(LayoutError::Malformed(
                    "InsertField index out of bounds".into(),
                ));
            }
        }
        Ok(base)
    }

    pub(super) fn repeat_phi_tree(
        &mut self,
        node: NodeId,
        ty: TyId,
        inputs: &[NodeId],
    ) -> Result<Option<Value>, LayoutError> {
        let TypeShape::Array { element, length } = self.graph.oracle.shape(ty) else {
            return Ok(None);
        };
        let (element, count) = (*element, *length);
        // Only explicit Repeat edges (and a direct identity backedge) establish
        // this representation. Never infer it from an Aggregate's equal inputs.
        if !inputs[1..].iter().all(|&input| {
            let input = self.graph.resolve(input);
            input == node || self.graph.op(input) == Opcode::Repeat
        }) {
            return Ok(None);
        }
        let parts = if count == 0 {
            Parts::Fields(Vec::new())
        } else {
            Parts::Repeat {
                element: Box::new(self.phi_tree(element, inputs[0], inputs.len())?),
                count,
                overrides: BTreeMap::new(),
            }
        };
        Ok(Some(Value { ty, parts }))
    }

    pub(super) fn encode_repeat(
        &mut self,
        element: &Value,
        count: u64,
        overrides: &BTreeMap<u32, Value>,
        address: NodeId,
        control: &mut NodeId,
        effect: &mut NodeId,
    ) -> Result<(), LayoutError> {
        let stride = self
            .graph
            .oracle
            .size_align(element.ty)
            .ok_or_else(|| LayoutError::Unsupported("unsized Repeat element".into()))?
            .size;
        let bytes = stride
            .checked_mul(count)
            .ok_or_else(|| LayoutError::Malformed("Repeat byte size overflow".into()))?;
        if count == 0 || stride == 0 {
            return Ok(());
        }
        if bytes <= self.options.max_unrolled_memory_bytes {
            for index in 0..count {
                let value = u32::try_from(index)
                    .ok()
                    .and_then(|i| overrides.get(&i))
                    .unwrap_or(element);
                let destination = self.field_address(address, index * stride, element.ty);
                self.encode_fields(value, destination, control, effect)?;
            }
        } else {
            // Fill only gaps. An overridden poison base must never transiently
            // overwrite a replacement, even when future lowering observes stores.
            let mut start = 0;
            for (&index, replacement) in overrides {
                let index = u64::from(index);
                self.encode_repeat_range(element, address, stride, start, index, control, effect)?;
                let destination = self.field_address(address, index * stride, element.ty);
                self.encode_fields(replacement, destination, control, effect)?;
                start = index + 1;
            }
            self.encode_repeat_range(element, address, stride, start, count, control, effect)?;
        }
        Ok(())
    }

    fn encode_repeat_range(
        &mut self,
        element: &Value,
        address: NodeId,
        stride: u64,
        start: u64,
        end: u64,
        control: &mut NodeId,
        effect: &mut NodeId,
    ) -> Result<(), LayoutError> {
        if start == end {
            return Ok(());
        }
        if end - start == 1 {
            let destination = self.field_address(address, start * stride, element.ty);
            return self.encode_fields(element, destination, control, effect);
        }
        let word = self.usize_type();
        let start = self.graph.add_constant(u128::from(start), word);
        let end = self.graph.add_constant(u128::from(end), word);
        let entry = self
            .graph
            .add_node(Opcode::Jump, *control, &[], 0, ValueType::Control);
        let region =
            self.graph
                .add_growable_node(Opcode::Region, NodeId::NONE, 2, 0, ValueType::Control);
        self.graph.set_input(region, 0, entry);
        let index =
            self.graph
                .add_growable_node(Opcode::Phi, NodeId::NONE, 3, 0, ValueType::Rust(word));
        self.graph.set_input(index, 0, region);
        self.graph.set_input(index, 1, start);
        let memory =
            self.graph
                .add_growable_node(Opcode::Phi, NodeId::NONE, 3, 0, ValueType::Effect);
        self.graph.set_input(memory, 0, region);
        self.graph.set_input(memory, 1, *effect);
        let boolean = self
            .graph
            .oracle
            .intern(TypeShape::Bool, Some(SizeAlign { size: 1, align: 1 }));
        let condition = self.graph.add_node(
            Opcode::Compare,
            NodeId::NONE,
            &[index, end],
            ComparePredicate::UnsignedLess as u32,
            ValueType::Rust(boolean),
        );
        let (mut body, exit) = self.split(region, condition);
        let stride = self.graph.add_constant(u128::from(stride), word);
        let offset = self.graph.add_node(
            Opcode::Mul,
            NodeId::NONE,
            &[index, stride],
            0,
            ValueType::Rust(word),
        );
        let pointer = self.pointer_type(element.ty);
        let destination = self.graph.add_node(
            Opcode::PtrOffset,
            NodeId::NONE,
            &[address, offset],
            0,
            ValueType::Rust(pointer),
        );
        let mut body_effect = memory;
        self.encode_fields(element, destination, &mut body, &mut body_effect)?;
        let one = self.graph.add_constant(1, word);
        let next = self.graph.add_node(
            Opcode::Add,
            NodeId::NONE,
            &[index, one],
            0,
            ValueType::Rust(word),
        );
        let backedge = self
            .graph
            .add_node(Opcode::Jump, body, &[], 0, ValueType::Control);
        self.graph.set_input(region, 1, backedge);
        self.graph.set_input(index, 2, next);
        self.graph.set_input(memory, 2, body_effect);
        *control = exit;
        *effect = memory;
        self.stats.repeat_loops += 1;
        self.stats.scalar_phis += 1;
        Ok(())
    }
}
