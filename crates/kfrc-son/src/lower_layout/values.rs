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
        let demand = self.demand(node);
        if demand.is_none() {
            return Ok(self.omitted(ty));
        }
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
                let scalar = self.known_variant_scalar(ty, variant)?;
                let fields = inputs
                    .into_iter()
                    .map(|input| self.expand(input))
                    .collect::<Result<_, _>>()?;
                Parts::KnownVariant {
                    variant,
                    scalar,
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
                    Parts::KnownVariant { scalar, .. } => scalar,
                    Parts::EncodedVariant(raw) => self.decode_variant_scalar(raw, source.ty)?,
                    _ => {
                        return Err(LayoutError::Malformed(
                            "discriminant operand is not enum".into(),
                        ));
                    }
                };
                let value = if op == Opcode::VariantOf {
                    let variant = self.variant_index(variant, source.ty)?;
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
                if self.owned_transport.contains(&node) {
                    self.stats.owned_transport_phis += 1;
                    let pointer = self.pointer_type(ty);
                    let address = self.graph.add_growable_node(
                        Opcode::Phi,
                        NodeId::NONE,
                        inputs.len(),
                        0,
                        ValueType::Rust(pointer),
                    );
                    self.graph.set_input(address, 0, inputs[0]);
                    let value = Value {
                        ty,
                        parts: Parts::OwnedStorage(address),
                    };
                    self.values.insert(node, value.clone());
                    for (slot, &incoming) in inputs.iter().enumerate().skip(1) {
                        let source = self.expand(incoming)?;
                        let source_address = if let Parts::OwnedStorage(address) = source.parts {
                            address
                        } else {
                            let destination = self.opaque_slot(ty)?;
                            self.owned_phis.push(owned_transport::OwnedPhiInit {
                                region: inputs[0],
                                slot,
                                source,
                                destination,
                            });
                            destination
                        };
                        self.graph.set_input(address, slot, source_address);
                    }
                    return Ok(value);
                }
                let template = match self.repeat_phi_tree(node, ty, &inputs)? {
                    Some(template) => template,
                    None => self.phi_tree_demand(ty, inputs[0], inputs.len(), &demand)?,
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
            Opcode::Undefined if self.composite(ty) => self.undefined_demand(ty, &demand)?.parts,
            Opcode::Projection if self.composite(ty) => {
                return self.decode_boundary(node, ty);
            }
            // Target-independent integer intrinsics retain their opcode,
            // operands and poison payload through the layout boundary.
            op if op.integer_intrinsic_arity().is_some() => Parts::Scalar(node),
            _ if !self.composite(ty) => Parts::Scalar(node),
            _ => {
                return Err(LayoutError::Unsupported(format!(
                    "aggregate {} at {node}",
                    op.name()
                )));
            }
        };
        if matches!(parts, Parts::Omitted) {
            return Err(LayoutError::Malformed(format!(
                "demanded value {node} was omitted"
            )));
        }
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
        self.undefined_demand(ty, &Demand::All)
    }

    pub(super) fn undefined_demand(
        &mut self,
        ty: TyId,
        demand: &Demand,
    ) -> Result<Value, LayoutError> {
        let demand = self.demand_for_type(ty, demand.clone());
        if demand.is_none() {
            return Ok(self.omitted(ty));
        }
        let parts = if self
            .graph
            .oracle
            .semantic()
            .maybe_uninit_inner(ty)
            .is_some()
        {
            Parts::OpaqueStorage(self.opaque_slot(ty)?)
        } else if let Some(plan) = self
            .graph
            .oracle
            .semantic()
            .is_enum(ty)
            .then(|| self.lane_plan(ty))
            .flatten()
        {
            let mut lanes = BTreeMap::new();
            let variants = self.lane_variants(
                ty,
                &demand,
                &plan,
                &mut lanes,
                |this, field, child| this.undefined_demand(field, child),
                &mut variant_lanes::LaneSource::Undefined,
            )?;
            let scalar_ty = self.variant_scalar_type(ty)?;
            let scalar = self.graph.add_node(
                Opcode::Undefined,
                NodeId::NONE,
                &[],
                0,
                ValueType::Rust(scalar_ty),
            );
            let variant = self.variant_scalar(ty, scalar);
            Parts::Variant { variant, variants }
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
                        .enumerate()
                        .map(|(field_index, field)| {
                            self.undefined_demand(
                                field,
                                &demand.child(DemandKey::VariantField(
                                    index as u32,
                                    field_index as u32,
                                )),
                            )
                        })
                        .collect::<Result<_, _>>()?,
                );
            }
            let scalar_ty = self.variant_scalar_type(ty)?;
            let scalar = self.graph.add_node(
                Opcode::Undefined,
                NodeId::NONE,
                &[],
                0,
                ValueType::Rust(scalar_ty),
            );
            let variant = self.variant_scalar(ty, scalar);
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
                    .enumerate()
                    .map(|(index, field)| {
                        self.undefined_demand(field, &demand.child(DemandKey::Field(index as u32)))
                    })
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
        self.phi_tree_demand(ty, region, arity, &Demand::All)
    }

    pub(super) fn phi_tree_demand(
        &mut self,
        ty: TyId,
        region: NodeId,
        arity: usize,
        demand: &Demand,
    ) -> Result<Value, LayoutError> {
        let demand = self.demand_for_type(ty, demand.clone());
        if demand.is_none() {
            return Ok(self.omitted(ty));
        }
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
            let scalar_ty = self.variant_scalar_type(ty)?;
            let scalar = self.phi_tree(scalar_ty, region, arity)?;
            let Parts::Scalar(node) = scalar.parts else {
                unreachable!()
            };
            let variant = self.variant_scalar(ty, node);
            if let Some(plan) = self.lane_plan(ty) {
                let mut lanes = BTreeMap::new();
                let variants = self.lane_variants(
                    ty,
                    &demand,
                    &plan,
                    &mut lanes,
                    |this, field, child| this.phi_tree_demand(field, region, arity, child),
                    &mut variant_lanes::LaneSource::Phi { region, arity },
                )?;
                return Ok(Value {
                    ty,
                    parts: Parts::Variant { variant, variants },
                });
            }
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
                        .enumerate()
                        .map(|(field_index, field)| {
                            self.phi_tree_demand(
                                field,
                                region,
                                arity,
                                &demand.child(DemandKey::VariantField(
                                    index as u32,
                                    field_index as u32,
                                )),
                            )
                        })
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
                    .enumerate()
                    .map(|(index, field)| {
                        self.phi_tree_demand(
                            field,
                            region,
                            arity,
                            &demand.child(DemandKey::Field(index as u32)),
                        )
                    })
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
        if matches!(target.parts, Parts::Variant { .. })
            && let Some(plan) = self.lane_plan(target.ty)
        {
            return self.fill_lane_phi(target, source, slot, &plan);
        }
        match (&target.parts, &source.parts) {
            (Parts::Omitted, _) => {}
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
                    scalar,
                    fields,
                },
            ) => {
                self.graph
                    .set_input(target_index.node(), slot, scalar.node());
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
                self.graph
                    .set_input(target_index.node(), slot, source_index.node());
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
    pub(super) fn fill_phi_poison(
        &mut self,
        target: &Value,
        slot: usize,
    ) -> Result<(), LayoutError> {
        match &target.parts {
            Parts::Omitted => {}
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
                    .rust_type(variant.node())
                    .ok_or_else(|| LayoutError::Malformed("variant Phi lacks index type".into()))?;
                let poison = self.graph.add_node(
                    Opcode::Undefined,
                    NodeId::NONE,
                    &[],
                    0,
                    ValueType::Rust(ty),
                );
                self.graph.set_input(variant.node(), slot, poison);
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
