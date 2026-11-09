//! Lower structured places and fixed semantic memory operations.
use super::*;
use crate::CallTarget;
use kfrc_body::{FunctionAbi, PassMode};

impl LayoutLowering<'_> {
    fn pointee(&self, pointer: NodeId) -> Result<TyId, LayoutError> {
        let ty = self
            .graph
            .rust_type(self.graph.resolve(pointer))
            .ok_or_else(|| LayoutError::Malformed("address lacks pointer type".into()))?;
        self.graph
            .oracle
            .semantic()
            .thin_pointee(ty)
            .ok_or_else(|| {
                LayoutError::Malformed(
                    "memory address is not an internal address or thin pointer".into(),
                )
            })
    }

    pub(super) fn lower_address(&mut self, node: NodeId) -> Result<NodeId, LayoutError> {
        let op = self.graph.op(node);
        if op == Opcode::LocalSlot {
            let ty = self.pointee(node)?;
            let layout = self
                .graph
                .oracle
                .size_align(ty)
                .ok_or_else(|| LayoutError::Unsupported("unsized local storage".into()))?;
            let slot = self.graph.stack_slots.len() as u32;
            self.graph.stack_slots.push(layout);
            self.graph
                .stack_slot_locals
                .insert(slot, self.graph.payload(node));
            self.graph.set_opcode_payload(node, Opcode::StackSlot, slot);
            return Ok(node);
        }
        let inputs = self.graph.inputs(node).to_vec();
        // Scalar expansion may expose a differently typed data pointer. Keep
        // the semantic owner's type for field offsets before that expansion.
        let owner = if matches!(op, Opcode::FieldAddr | Opcode::VariantFieldAddr) {
            Some(self.pointee(inputs[0])?)
        } else {
            None
        };
        let base = self.expand(inputs[0])?;
        let Parts::Scalar(base) = base.parts else {
            return Err(LayoutError::Malformed(
                "structured address base is not thin pointer".into(),
            ));
        };
        let usize_ty = self.usize_type();
        let offset = match op {
            Opcode::FieldAddr | Opcode::VariantFieldAddr => {
                let (variant, index) = if op == Opcode::VariantFieldAddr {
                    let (variant, field) =
                        self.graph.variant_fields[self.graph.payload(node) as usize];
                    (Some(variant), field)
                } else {
                    (None, self.graph.payload(node))
                };
                let fields = self.fields(owner.unwrap(), variant)?;
                let field = fields.get(index as usize).ok_or_else(|| {
                    LayoutError::Malformed("field address index out of bounds".into())
                })?;
                self.graph.add_constant(u128::from(field.offset), usize_ty)
            }
            Opcode::IndexAddr => {
                let element = self.pointee(node)?;
                let size = self
                    .graph
                    .oracle
                    .size_align(element)
                    .ok_or_else(|| LayoutError::Unsupported("unsized indexed element".into()))?
                    .size;
                let index = self.expand(inputs[1])?;
                let Parts::Scalar(index) = index.parts else {
                    return Err(LayoutError::Malformed("aggregate index".into()));
                };
                // Pointer offsets wrap at pointer width. In particular, an
                // isize index must be retyped before unsigned stride arithmetic.
                let index = if self.graph.rust_type(index) == Some(usize_ty) {
                    index
                } else {
                    self.graph.add_node(
                        Opcode::IntCast,
                        NodeId::NONE,
                        &[index],
                        0,
                        ValueType::Rust(usize_ty),
                    )
                };
                let size = self.graph.add_constant(u128::from(size), usize_ty);
                self.graph.add_node(
                    Opcode::Mul,
                    NodeId::NONE,
                    &[index, size],
                    0,
                    ValueType::Rust(usize_ty),
                )
            }
            Opcode::PointerCast => self.graph.add_constant(0, usize_ty),
            _ => {
                return Err(LayoutError::Malformed(
                    "not a structured address operation".into(),
                ));
            }
        };
        self.graph.set_opcode_payload(node, Opcode::PtrOffset, 0);
        self.graph.set_inputs(node, &[base, offset]);
        Ok(node)
    }

    pub(super) fn lower_load_value(&mut self, node: NodeId) -> Result<(), LayoutError> {
        if self.graph.is_dead(node) {
            return Ok(());
        }
        // A nonvolatile typed read whose result has no execution consumer
        // contributes only a source-domain requirement. Erase its ordering
        // link without decoding payloads; used snapshots keep their read point.
        if self.projections.find(self.graph, node, 2).is_none() {
            let control = self.graph.resolve(self.graph.control(node));
            let effect = self.graph.resolve(self.graph.input(node, 0));
            self.bypass_fixed(node, control, effect);
            return Ok(());
        }
        let ty = self.pointee(self.graph.input(node, 1))?;
        let address = self.expand(self.graph.input(node, 1))?;
        let Parts::Scalar(address) = address.parts else {
            return Err(LayoutError::Malformed("load address is aggregate".into()));
        };
        let mut control = self.graph.resolve(self.graph.control(node));
        let mut effect = self.graph.resolve(self.graph.input(node, 0));
        let previous_partition = self.partition;
        self.partition = self.graph.payload(node);
        let result = self.projections.find(self.graph, node, 2);
        let value = if result.is_some_and(|value| self.encoded_values.contains(&value)) {
            self.decode_encoded(address, ty, &mut control, &mut effect)?
        } else if self.tag_only_loads.contains(&node) {
            Value {
                ty,
                parts: Parts::VariantTag(self.read_variant_index(
                    address,
                    ty,
                    &mut control,
                    &mut effect,
                )?),
            }
        } else {
            self.decode_fields(address, ty, &mut control, &mut effect)?
        };
        self.partition = previous_partition;
        if let Some(result) = self.projections.find(self.graph, node, 2) {
            self.values.insert(result, value.clone());
            if let Parts::Scalar(scalar) = value.parts {
                self.graph.replace(result, scalar);
            } else {
                self.retired.push(result);
            }
        }
        self.bypass_fixed(node, control, effect);
        Ok(())
    }

    pub(super) fn bypass_fixed(&mut self, node: NodeId, control: NodeId, effect: NodeId) {
        if let Some(old) = self.projections.find(self.graph, node, 0) {
            self.graph.replace(old, control);
        }
        if let Some(old) = self.projections.find(self.graph, node, 1) {
            self.graph.replace(old, effect);
        }
        self.graph.remove_node(node);
    }

    pub(super) fn lower_memory_effect(&mut self, node: NodeId) -> Result<(), LayoutError> {
        if self.graph.is_dead(node) {
            return Ok(());
        }
        let op = self.graph.op(node);
        let mut control = self.graph.resolve(self.graph.control(node));
        let mut effect = self.graph.resolve(self.graph.input(node, 0));
        let previous_partition = self.partition;
        self.partition = self.graph.payload(node);
        match op {
            Opcode::LoadValue => return self.lower_load_value(node),
            Opcode::TransmuteValue => return self.lower_transmute(node),
            Opcode::StoreValue => {
                let address = self.expand(self.graph.input(node, 1))?;
                let Parts::Scalar(address) = address.parts else {
                    return Err(LayoutError::Malformed("store address is aggregate".into()));
                };
                let value = self.expand(self.graph.input(node, 2))?;
                self.encode_fields(&value, address, &mut control, &mut effect)?;
            }
            Opcode::SetVariant => {
                let ty = self.pointee(self.graph.input(node, 1))?;
                let address = self.expand(self.graph.input(node, 1))?;
                let Parts::Scalar(address) = address.parts else {
                    return Err(LayoutError::Malformed(
                        "variant address is aggregate".into(),
                    ));
                };
                let index = self.graph.resolve(self.graph.input(node, 2));
                if self.graph.op(index) != Opcode::Const {
                    return Err(LayoutError::Unsupported("nonconstant SetVariant".into()));
                }
                let variant = u32::try_from(self.graph.constant_value(index))
                    .map_err(|_| LayoutError::Malformed("variant index exceeds u32".into()))?;
                self.write_variant_tag(address, ty, variant, &mut control, &mut effect)?;
            }
            Opcode::CopyNonOverlapping => {
                let element = self.pointee(self.graph.input(node, 1))?;
                let mut arguments = Vec::new();
                for index in 1..=3 {
                    let value = self.expand(self.graph.input(node, index))?;
                    let Parts::Scalar(value) = value.parts else {
                        return Err(LayoutError::Malformed("copy operand is aggregate".into()));
                    };
                    arguments.push(value);
                }
                let size = self
                    .graph
                    .oracle
                    .size_align(element)
                    .ok_or_else(|| LayoutError::Unsupported("unsized copy element".into()))?
                    .size;
                let usize_ty = self.usize_type();
                // S permits any integer count; byte arithmetic is at target
                // pointer width, including a distinct same-width u64 type.
                let count = if self.graph.rust_type(arguments[2]) == Some(usize_ty) {
                    arguments[2]
                } else {
                    self.graph.add_node(
                        Opcode::IntCast,
                        NodeId::NONE,
                        &[arguments[2]],
                        0,
                        ValueType::Rust(usize_ty),
                    )
                };
                let size = self.graph.add_constant(u128::from(size), usize_ty);
                let bytes = self.graph.add_node(
                    Opcode::Mul,
                    NodeId::NONE,
                    &[count, size],
                    0,
                    ValueType::Rust(usize_ty),
                );
                let target = self.graph.add_callee(CallTarget {
                    symbol: "memcpy".into(),
                    abi: FunctionAbi {
                        args: vec![PassMode::Direct; 3],
                        ret: PassMode::Direct,
                        c_variadic: false,
                    },
                });
                let result_ty = self.graph.value_type(arguments[1]).clone();
                let call = self.graph.add_node(
                    Opcode::Call,
                    control,
                    &[effect, arguments[1], arguments[0], bytes],
                    target,
                    ValueType::Tuple(vec![ValueType::Control, ValueType::Effect, result_ty]),
                );
                control = self.projection(call, 0, ValueType::Control);
                effect = self.projection(call, 1, ValueType::Effect);
            }
            Opcode::SlotLive | Opcode::SlotDead => {
                // Lifetime markers remain ordered through machine scheduling.
                return Ok(());
            }
            _ => return Ok(()),
        }
        self.partition = previous_partition;
        self.bypass_fixed(node, control, effect);
        Ok(())
    }
}
