//! MaybeUninit is an owned raw snapshot, never a typed read of its inner value.
use super::*;
use crate::CallTarget;
use kfrc_body::{DataObject, FunctionAbi, PassMode};

#[derive(Clone, Copy)]
pub(super) struct OpaquePhiCopy {
    pub region: NodeId,
    pub slot: usize,
    pub source: NodeId,
    pub destination: NodeId,
    pub ty: TyId,
}

impl LayoutLowering<'_> {
    pub(super) fn opaque_slot(&mut self, ty: TyId) -> Result<NodeId, LayoutError> {
        let layout = self
            .graph
            .oracle
            .size_align(ty)
            .ok_or_else(|| LayoutError::Unsupported("unsized MaybeUninit".into()))?;
        let slot = self.graph.stack_slots.len() as u32;
        self.graph.stack_slots.push(layout);
        let pointer = self.pointer_type(ty);
        Ok(self.graph.add_node(
            Opcode::StackSlot,
            NodeId::NONE,
            &[],
            slot,
            ValueType::Rust(pointer),
        ))
    }

    pub(super) fn raw_copy(
        &mut self,
        source: NodeId,
        destination: NodeId,
        size: u64,
        control: &mut NodeId,
        effect: &mut NodeId,
    ) {
        if size == 0 || source == destination {
            return;
        }
        let byte = self.graph.oracle.intern(
            TypeShape::Int {
                width: IntegerWidth::W8,
                signed: false,
            },
            Some(SizeAlign { size: 1, align: 1 }),
        );
        let source = self.field_address(source, 0, byte);
        let destination = self.field_address(destination, 0, byte);
        let word = self.usize_type();
        let count = self.graph.add_constant(u128::from(size), word);
        let callee = self.graph.add_callee(CallTarget {
            symbol: "memmove".into(),
            abi: FunctionAbi {
                args: vec![PassMode::Direct; 3],
                ret: PassMode::Direct,
                c_variadic: false,
            },
        });
        let result = self.graph.value_type(destination).clone();
        let call = self.graph.add_node(
            Opcode::Call,
            *control,
            &[*effect, destination, source, count],
            callee,
            ValueType::Tuple(vec![ValueType::Control, ValueType::Effect, result]),
        );
        *control = self.projection(call, 0, ValueType::Control);
        *effect = self.projection(call, 1, ValueType::Effect);
    }

    pub(super) fn decode_opaque(
        &mut self,
        source: NodeId,
        ty: TyId,
        control: &mut NodeId,
        effect: &mut NodeId,
    ) -> Result<Value, LayoutError> {
        let destination = self.opaque_slot(ty)?;
        let size = self.graph.oracle.size_align(ty).unwrap().size;
        self.raw_copy(source, destination, size, control, effect);
        Ok(Value {
            ty,
            parts: Parts::OpaqueStorage(destination),
        })
    }

    pub(super) fn opaque_constant(
        &mut self,
        ty: TyId,
        bits: u128,
        offset: u64,
    ) -> Result<Value, LayoutError> {
        let layout = self
            .graph
            .oracle
            .size_align(ty)
            .ok_or_else(|| LayoutError::Unsupported("unsized MaybeUninit constant".into()))?;
        if offset.checked_add(layout.size).is_none_or(|end| end > 16) {
            return Err(LayoutError::Unsupported(
                "MaybeUninit representation constant exceeds 128 bits".into(),
            ));
        }
        let bytes = bits.to_le_bytes()[offset as usize..(offset + layout.size) as usize].to_vec();
        let symbol = format!(
            "__kfrc_mu_{}_{}",
            self.graph.symbol,
            self.graph.data_objects.len()
        );
        self.graph.data_objects.push(DataObject {
            symbol: symbol.clone(),
            bytes,
            align: layout.align,
            ..DataObject::default()
        });
        let payload = self.graph.symbols.len() as u32;
        self.graph.symbols.push((symbol, 0));
        let pointer = self.pointer_type(ty);
        let address = self.graph.add_node(
            Opcode::SymbolAddress,
            NodeId::NONE,
            &[],
            payload,
            ValueType::Rust(pointer),
        );
        Ok(Value {
            ty,
            parts: Parts::OpaqueStorage(address),
        })
    }

    // Recover the incoming effect at a control point. Publish Region Phis before
    // visiting backedges; fixed raw copies are ordered by control as well.
    pub(super) fn opaque_effect_at(
        &mut self,
        control: NodeId,
        cache: &mut HashMap<NodeId, NodeId>,
    ) -> Result<NodeId, LayoutError> {
        let control = self.graph.resolve(control);
        if let Some(&effect) = cache.get(&control) {
            return Ok(effect);
        }
        let effect = match self.graph.op(control) {
            Opcode::Projection => {
                let source = self.graph.input(control, 0);
                if let ValueType::Tuple(types) = self.graph.value_type(source)
                    && types.get(1) == Some(&ValueType::Effect)
                {
                    self.projection(source, 1, ValueType::Effect)
                } else {
                    self.opaque_effect_at(self.graph.control(source), cache)?
                }
            }
            Opcode::Region => {
                let existing = self.graph.ids().find(|&node| {
                    self.graph.op(node) == Opcode::Phi
                        && self.graph.input(node, 0) == control
                        && self.graph.value_type(node) == &ValueType::Effect
                });
                if let Some(effect) = existing {
                    effect
                } else {
                    let predecessors = self.graph.inputs(control).to_vec();
                    let phi = self.graph.add_growable_node(
                        Opcode::Phi,
                        NodeId::NONE,
                        predecessors.len() + 1,
                        0,
                        ValueType::Effect,
                    );
                    self.graph.set_input(phi, 0, control);
                    cache.insert(control, phi);
                    for (index, jump) in predecessors.into_iter().enumerate() {
                        let effect = self.opaque_effect_at(self.graph.control(jump), cache)?;
                        self.graph.set_input(phi, index + 1, effect);
                    }
                    phi
                }
            }
            _ => {
                return Err(LayoutError::Malformed(
                    "opaque Phi predecessor has no effect anchor".into(),
                ));
            }
        };
        cache.insert(control, effect);
        Ok(effect)
    }
}
