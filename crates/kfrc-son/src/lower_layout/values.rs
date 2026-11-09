//! Scalar components retain SSA identity across aggregate construction and Phi.

use super::*;

impl LayoutLowering<'_> {
    pub(super) fn expand(&mut self, node: NodeId) -> Result<Value, LayoutError> {
        let node = self.graph.resolve(node);
        if let Some(value) = self.values.get(&node) {
            return Ok(value.clone());
        }
        let ty = self
            .graph
            .rust_type(node)
            .ok_or_else(|| LayoutError::Malformed(format!("{node} is not a value")))?;
        let op = self.graph.op(node);
        let inputs = self.graph.inputs(node).to_vec();
        let parts = match op {
            Opcode::ConstRepr => {
                let value = self.repr_value(ty, self.graph.constant_value(node), 0)?;
                self.values.insert(node, value.clone());
                return Ok(value);
            }
            Opcode::Projection if self.graph.op(inputs[0]) == Opcode::TransmuteValue => {
                self.lower_transmute(inputs[0])?;
                return self.values.get(&node).cloned().ok_or_else(|| {
                    LayoutError::Malformed("transmute has no value projection".into())
                });
            }

            Opcode::LocalSlot
            | Opcode::FieldAddr
            | Opcode::VariantFieldAddr
            | Opcode::IndexAddr
            | Opcode::PointerCast => Parts::Scalar(self.lower_address(node)?),
            Opcode::Projection if self.graph.op(inputs[0]) == Opcode::LoadValue => {
                self.lower_load_value(inputs[0])?;
                return self.values.get(&node).cloned().ok_or_else(|| {
                    LayoutError::Malformed("semantic load has no value projection".into())
                });
            }
            Opcode::Aggregate if matches!(self.graph.oracle.shape(ty), TypeShape::Unit) => {
                Parts::Scalar(self.graph.add_constant(0, ty))
            }
            Opcode::Aggregate | Opcode::WidePtr => Parts::Fields(
                inputs
                    .into_iter()
                    .map(|input| self.expand(input))
                    .collect::<Result<_, _>>()?,
            ),
            Opcode::Repeat => {
                let TypeShape::Array { length, .. } = self.graph.oracle.shape(ty) else {
                    return Err(LayoutError::Malformed(
                        "Repeat result is not an array".into(),
                    ));
                };
                let count = *length;
                if count == 0 {
                    Parts::Fields(Vec::new())
                } else {
                    Parts::Repeat {
                        element: Box::new(self.expand(inputs[0])?),
                        count,
                        overrides: BTreeMap::new(),
                    }
                }
            }
            Opcode::MakeVariant => {
                let variant = self.graph.payload(node);
                let index_ty = self.usize_type();
                let index = self.graph.add_constant(u128::from(variant), index_ty);
                let fields = inputs
                    .into_iter()
                    .map(|input| self.expand(input))
                    .collect::<Result<_, _>>()?;
                Parts::KnownVariant {
                    variant,
                    index,
                    fields,
                }
            }
            Opcode::Field => {
                let base = self.expand(inputs[0])?;
                self.value_field(&base, self.graph.payload(node))?.parts
            }
            Opcode::InsertField => {
                let base = self.expand(inputs[0])?;
                let replacement = self.expand(inputs[1])?;
                self.insert_value_field(base, self.graph.payload(node), replacement)?
                    .parts
            }
            Opcode::VariantField => {
                let (expected, field) =
                    self.graph.variant_fields[self.graph.payload(node) as usize];
                let base = self.expand(inputs[0])?;
                if let Parts::KnownVariant {
                    variant, fields, ..
                } = &base.parts
                {
                    if *variant == expected {
                        fields
                            .get(field as usize)
                            .ok_or_else(|| {
                                LayoutError::Malformed(
                                    "known variant field index out of bounds".into(),
                                )
                            })?
                            .parts
                            .clone()
                    } else {
                        // Inactive extraction is poison, not an observation or
                        // a read of the active variant's bytes.
                        self.undefined(ty)?.parts
                    }
                } else if let Parts::EncodedVariant(raw) = base.parts {
                    return self.encoded_field(node, base.ty, raw, expected, field);
                } else {
                    let Parts::Variant { variants, .. } = base.parts else {
                        return Err(LayoutError::Malformed(
                            "VariantField operand is not enum".into(),
                        ));
                    };
                    variants
                        .get(expected as usize)
                        .and_then(|fields| fields.get(field as usize))
                        .cloned()
                        .ok_or_else(|| {
                            LayoutError::Malformed("variant field index out of bounds".into())
                        })?
                        .parts
                }
            }
            Opcode::VariantOf | Opcode::Discriminant => {
                let source = self.expand(inputs[0])?;
                let variant = match source.parts {
                    Parts::Variant { variant, .. } | Parts::VariantTag(variant) => variant,
                    Parts::KnownVariant { index, .. } => index,
                    Parts::EncodedVariant(raw) => self.decode_tag_index(raw, source.ty)?,
                    _ => {
                        return Err(LayoutError::Malformed(
                            "discriminant operand is not enum".into(),
                        ));
                    }
                };
                let value = if op == Opcode::VariantOf {
                    self.graph.add_node(
                        Opcode::IntCast,
                        NodeId::NONE,
                        &[variant],
                        0,
                        ValueType::Rust(ty),
                    )
                } else {
                    self.source_discriminant(source.ty, variant, ty)?
                };
                Parts::Scalar(value)
            }
            Opcode::Phi if self.composite(ty) => {
                let template = match self.repeat_phi_tree(node, ty, &inputs)? {
                    Some(template) => template,
                    None => self.phi_tree(ty, inputs[0], inputs.len())?,
                };
                // Publish placeholders before following backedges. Each field
                // is a real Phi; no loop carries an address to reused storage.
                self.values.insert(node, template.clone());
                for (slot, input) in inputs.iter().enumerate().skip(1) {
                    let incoming = self.expand(*input)?;
                    self.fill_phi(&template, &incoming, slot)?;
                }
                return Ok(template);
            }
            Opcode::Undefined if self.composite(ty) => self.undefined(ty)?.parts,
            Opcode::Projection if self.composite(ty) => {
                return self.decode_boundary(node, ty);
            }
            _ if !self.composite(ty) => Parts::Scalar(node),
            _ => {
                return Err(LayoutError::Unsupported(format!(
                    "aggregate {} at {node}",
                    op.name()
                )));
            }
        };
        let value = Value { ty, parts };
        if let Parts::Scalar(scalar) = value.parts {
            if op.is_semantic_value() {
                self.graph.replace(node, scalar);
            }
        }
        self.values.insert(node, value.clone());
        Ok(value)
    }

    pub(super) fn undefined(&mut self, ty: TyId) -> Result<Value, LayoutError> {
        let parts = if self
            .graph
            .oracle
            .semantic()
            .maybe_uninit_inner(ty)
            .is_some()
        {
            Parts::OpaqueStorage(self.opaque_slot(ty)?)
        } else if self.graph.oracle.semantic().is_enum(ty) {
            let count = self.graph.oracle.semantic().variant_count(ty).unwrap_or(0);
            let mut variants = Vec::new();
            for index in 0..count {
                let fields = self
                    .graph
                    .oracle
                    .semantic()
                    .field_types(ty, Some(index as u32))
                    .ok_or_else(|| LayoutError::Malformed("enum variant has no fields".into()))?;
                variants.push(
                    fields
                        .into_iter()
                        .map(|field| self.undefined(field))
                        .collect::<Result<_, _>>()?,
                );
            }
            let index_ty = self.usize_type();
            let variant = self.graph.add_node(
                Opcode::Undefined,
                NodeId::NONE,
                &[],
                0,
                ValueType::Rust(index_ty),
            );
            Parts::Variant { variant, variants }
        } else if let TypeShape::Array { element, length } = self.graph.oracle.shape(ty) {
            let (element, count) = (*element, *length);
            if count == 0 {
                Parts::Fields(Vec::new())
            } else {
                Parts::Repeat {
                    element: Box::new(self.undefined(element)?),
                    count,
                    overrides: BTreeMap::new(),
                }
            }
        } else if self.composite(ty) {
            let fields = self
                .graph
                .oracle
                .semantic()
                .field_types(ty, None)
                .ok_or_else(|| LayoutError::Malformed("aggregate has no logical fields".into()))?;
            Parts::Fields(
                fields
                    .into_iter()
                    .map(|field| self.undefined(field))
                    .collect::<Result<_, _>>()?,
            )
        } else {
            Parts::Scalar(self.graph.add_node(
                Opcode::Undefined,
                NodeId::NONE,
                &[],
                0,
                ValueType::Rust(ty),
            ))
        };
        Ok(Value { ty, parts })
    }

    pub(super) fn phi_tree(
        &mut self,
        ty: TyId,
        region: NodeId,
        arity: usize,
    ) -> Result<Value, LayoutError> {
        let parts = if self
            .graph
            .oracle
            .semantic()
            .maybe_uninit_inner(ty)
            .is_some()
        {
            let address = self.opaque_slot(ty)?;
            self.opaque_phi_regions.insert(address, region);
            Parts::OpaqueStorage(address)
        } else if self.graph.oracle.semantic().is_enum(ty) {
            let index_ty = self.usize_type();
            let index = self.phi_tree(index_ty, region, arity)?;
            let Parts::Scalar(variant) = index.parts else {
                unreachable!()
            };
            let count = self.graph.oracle.semantic().variant_count(ty).unwrap_or(0);
            let mut variants = Vec::new();
            for index in 0..count {
                let fields = self
                    .graph
                    .oracle
                    .semantic()
                    .field_types(ty, Some(index as u32))
                    .unwrap();
                variants.push(
                    fields
                        .into_iter()
                        .map(|field| self.phi_tree(field, region, arity))
                        .collect::<Result<_, _>>()?,
                );
            }
            Parts::Variant { variant, variants }
        } else if self.composite(ty) {
            let fields = self
                .graph
                .oracle
                .semantic()
                .field_types(ty, None)
                .ok_or_else(|| LayoutError::Malformed("Phi aggregate has no fields".into()))?;
            Parts::Fields(
                fields
                    .into_iter()
                    .map(|field| self.phi_tree(field, region, arity))
                    .collect::<Result<_, _>>()?,
            )
        } else {
            let phi = self.graph.add_growable_node(
                Opcode::Phi,
                NodeId::NONE,
                arity,
                0,
                ValueType::Rust(ty),
            );
            self.graph.set_input(phi, 0, region);
            self.stats.scalar_phis += 1;
            Parts::Scalar(phi)
        };
        Ok(Value { ty, parts })
    }

    pub(super) fn fill_phi(
        &mut self,
        target: &Value,
        source: &Value,
        slot: usize,
    ) -> Result<(), LayoutError> {
        if target.ty != source.ty {
            return Err(LayoutError::Malformed("Phi field type mismatch".into()));
        }
        match (&target.parts, &source.parts) {
            (Parts::OpaqueStorage(destination), Parts::OpaqueStorage(source)) => {
                let region = *self.opaque_phi_regions.get(destination).ok_or_else(|| {
                    LayoutError::Malformed("opaque Phi destination lacks region".into())
                })?;
                self.opaque_phis.push(maybe_uninit::OpaquePhiCopy {
                    region,
                    slot,
                    source: *source,
                    destination: *destination,
                    ty: target.ty,
                });
            }
            (Parts::Scalar(phi), Parts::Scalar(value)) => self.graph.set_input(*phi, slot, *value),
            (Parts::Fields(target), Parts::Fields(source)) if target.len() == source.len() => {
                for (target, source) in target.iter().zip(source) {
                    self.fill_phi(target, source, slot)?;
                }
            }
            (Parts::Fields(targets), Parts::Repeat { count, .. })
                if targets.len() as u64 == *count =>
            {
                for (index, target) in targets.iter().enumerate() {
                    let source = self.value_field(
                        source,
                        u32::try_from(index).map_err(|_| {
                            LayoutError::Unsupported("array Phi field exceeds u32".into())
                        })?,
                    )?;
                    self.fill_phi(target, &source, slot)?;
                }
            }
            (
                Parts::Repeat {
                    element: target,
                    count: target_count,
                    overrides: target_overrides,
                },
                Parts::Repeat {
                    element: source,
                    count: source_count,
                    overrides: source_overrides,
                },
            ) if target_count == source_count
                && target_overrides.is_empty()
                && source_overrides.is_empty() =>
            {
                self.fill_phi(target, source, slot)?;
            }
            (
                Parts::Variant {
                    variant: target_index,
                    variants: targets,
                },
                Parts::KnownVariant {
                    variant,
                    index,
                    fields,
                },
            ) => {
                self.graph.set_input(*target_index, slot, *index);
                for (case, targets) in targets.iter().enumerate() {
                    if case == *variant as usize {
                        if targets.len() != fields.len() {
                            return Err(LayoutError::Malformed(
                                "known variant Phi field shape mismatch".into(),
                            ));
                        }
                        for (target, source) in targets.iter().zip(fields) {
                            self.fill_phi(target, source, slot)?;
                        }
                    } else {
                        for target in targets {
                            self.fill_phi_poison(target, slot)?;
                        }
                    }
                }
            }
            (
                Parts::Variant {
                    variant: target_index,
                    variants: targets,
                },
                Parts::Variant {
                    variant: source_index,
                    variants: sources,
                },
            ) if targets.len() == sources.len() => {
                self.graph.set_input(*target_index, slot, *source_index);
                for (target, source) in targets.iter().zip(sources) {
                    if target.len() != source.len() {
                        return Err(LayoutError::Malformed("variant Phi shape mismatch".into()));
                    }
                    for (target, source) in target.iter().zip(source) {
                        self.fill_phi(target, source, slot)?;
                    }
                }
            }
            _ => return Err(LayoutError::Malformed("Phi field shape mismatch".into())),
        }
        Ok(())
    }
}

impl LayoutLowering<'_> {
    /// Fill only leaves already required by a destination Phi. No temporary
    /// undefined aggregate tree is needed for a constructor's inactive case.
    fn fill_phi_poison(&mut self, target: &Value, slot: usize) -> Result<(), LayoutError> {
        match &target.parts {
            Parts::OpaqueStorage(_) => {
                let source = self.undefined(target.ty)?;
                self.fill_phi(target, &source, slot)?;
            }
            Parts::Scalar(phi) => {
                let poison = self.graph.add_node(
                    Opcode::Undefined,
                    NodeId::NONE,
                    &[],
                    0,
                    ValueType::Rust(target.ty),
                );
                self.graph.set_input(*phi, slot, poison);
            }
            Parts::Fields(fields) => {
                for field in fields {
                    self.fill_phi_poison(field, slot)?;
                }
            }
            Parts::Repeat {
                element, overrides, ..
            } => {
                self.fill_phi_poison(element, slot)?;
                for field in overrides.values() {
                    self.fill_phi_poison(field, slot)?;
                }
            }
            Parts::Variant { variant, variants } => {
                let ty = self
                    .graph
                    .rust_type(*variant)
                    .ok_or_else(|| LayoutError::Malformed("variant Phi lacks index type".into()))?;
                let poison = self.graph.add_node(
                    Opcode::Undefined,
                    NodeId::NONE,
                    &[],
                    0,
                    ValueType::Rust(ty),
                );
                self.graph.set_input(*variant, slot, poison);
                for field in variants.iter().flatten() {
                    self.fill_phi_poison(field, slot)?;
                }
            }
            _ => {
                return Err(LayoutError::Malformed(
                    "poison destination is not a Phi tree".into(),
                ));
            }
        }
        Ok(())
    }
}
