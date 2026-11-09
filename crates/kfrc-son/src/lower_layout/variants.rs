//! Enum values keep a domain-tagged scalar and guarded payload SSA values.
//! Inactive payload bytes are never decoded as typed values.
use super::*;
use crate::ComparePredicate;
use kfrc_body::TagEncoding;

impl LayoutLowering<'_> {
    pub(super) fn equal(&mut self, lhs: NodeId, rhs: NodeId) -> NodeId {
        let boolean = self
            .graph
            .oracle
            .intern(TypeShape::Bool, Some(SizeAlign { size: 1, align: 1 }));
        self.graph.add_node(
            Opcode::Compare,
            NodeId::NONE,
            &[lhs, rhs],
            ComparePredicate::Equal as u32,
            ValueType::Rust(boolean),
        )
    }

    pub(super) fn bool_type(&mut self) -> TyId {
        self.graph
            .oracle
            .intern(TypeShape::Bool, Some(SizeAlign { size: 1, align: 1 }))
    }

    pub(super) fn select_scalar(
        &mut self,
        condition: NodeId,
        yes: NodeId,
        no: NodeId,
        ty: TyId,
    ) -> NodeId {
        // Both alternatives are initialized tag integers. Payload poison never
        // passes through this arithmetic selection.
        if let Some(selected) = self.select_constant_scalar(condition, yes, no, ty) {
            return selected;
        }
        let flag = self.graph.add_node(
            Opcode::IntCast,
            NodeId::NONE,
            &[condition],
            0,
            ValueType::Rust(ty),
        );
        let mask = self.graph.add_node(
            Opcode::Negate,
            NodeId::NONE,
            &[flag],
            0,
            ValueType::Rust(ty),
        );
        if self.constant_bits(no) == Some(0) {
            // `(mask & yes) | (!mask & 0)` keeps only the first term.
            return self.graph.add_node(
                Opcode::BitAnd,
                NodeId::NONE,
                &[mask, yes],
                0,
                ValueType::Rust(ty),
            );
        }
        let inverse = self.graph.add_node(
            Opcode::BitNot,
            NodeId::NONE,
            &[mask],
            0,
            ValueType::Rust(ty),
        );
        let yes = self.graph.add_node(
            Opcode::BitAnd,
            NodeId::NONE,
            &[mask, yes],
            0,
            ValueType::Rust(ty),
        );
        let no = self.graph.add_node(
            Opcode::BitAnd,
            NodeId::NONE,
            &[inverse, no],
            0,
            ValueType::Rust(ty),
        );
        self.graph.add_node(
            Opcode::BitOr,
            NodeId::NONE,
            &[yes, no],
            0,
            ValueType::Rust(ty),
        )
    }

    fn constant_bits(&self, node: NodeId) -> Option<u128> {
        let node = self.graph.resolve(node);
        (self.graph.op(node) == Opcode::Const).then(|| self.graph.constant_value(node))
    }

    /// Selection between two constants of an unsigned integer type. A defined
    /// `bool` is exactly 0 or 1, so its integer cast is already the
    /// selection when the alternatives differ by one in the condition's order.
    fn select_constant_scalar(
        &mut self,
        condition: NodeId,
        yes: NodeId,
        no: NodeId,
        ty: TyId,
    ) -> Option<NodeId> {
        let (Some(yes_bits), Some(no_bits)) = (self.constant_bits(yes), self.constant_bits(no))
        else {
            return None;
        };
        if yes_bits == no_bits {
            return Some(no);
        }
        let TypeShape::Int {
            width,
            signed: false,
        } = *self.graph.oracle.shape(ty)
        else {
            return None;
        };
        let bits = u32::from(width.bits(64));
        if bits > 64 || yes_bits >> bits != 0 || no_bits >> bits != 0 {
            return None;
        }
        let flag = self.graph.add_node(
            Opcode::IntCast,
            NodeId::NONE,
            &[condition],
            0,
            ValueType::Rust(ty),
        );
        if yes_bits == no_bits.wrapping_add(1) {
            return Some(if no_bits == 0 {
                flag
            } else {
                self.graph.add_node(
                    Opcode::Add,
                    NodeId::NONE,
                    &[flag, no],
                    0,
                    ValueType::Rust(ty),
                )
            });
        }
        // `no ^ (mask & (yes ^ no))` with `mask = -flag`.
        let mask = self.graph.add_node(
            Opcode::Negate,
            NodeId::NONE,
            &[flag],
            0,
            ValueType::Rust(ty),
        );
        let difference = self.graph.add_constant(yes_bits ^ no_bits, ty);
        let picked = self.graph.add_node(
            Opcode::BitAnd,
            NodeId::NONE,
            &[mask, difference],
            0,
            ValueType::Rust(ty),
        );
        Some(if no_bits == 0 {
            picked
        } else {
            self.graph.add_node(
                Opcode::BitXor,
                NodeId::NONE,
                &[picked, no],
                0,
                ValueType::Rust(ty),
            )
        })
    }

    pub(super) fn source_discriminant(
        &mut self,
        ty: TyId,
        variant: VariantScalar,
        output_ty: TyId,
    ) -> Result<NodeId, LayoutError> {
        if let VariantScalar::Direct(raw) = variant {
            return Ok(self.graph.add_node(
                Opcode::IntCast,
                NodeId::NONE,
                &[raw],
                0,
                ValueType::Rust(output_ty),
            ));
        }
        let count = self
            .graph
            .oracle
            .semantic()
            .variant_count(ty)
            .ok_or_else(|| LayoutError::Malformed("enum has no variants".into()))?;
        let variant = self.graph.resolve(variant.node());
        if self.graph.op(variant) == Opcode::Const {
            let index = u32::try_from(self.graph.constant_value(variant))
                .map_err(|_| LayoutError::Malformed("variant index exceeds u32".into()))?;
            let bits = self
                .graph
                .oracle
                .semantic()
                .discriminant(ty, index)
                .ok_or_else(|| LayoutError::Malformed("enum missing source discriminant".into()))?;
            return Ok(self.graph.add_constant(bits, output_ty));
        }
        if (0..count).all(|index| {
            self.graph.oracle.semantic().discriminant(ty, index as u32) == Some(index as u128)
        }) {
            return Ok(self.graph.add_node(
                Opcode::IntCast,
                NodeId::NONE,
                &[variant],
                0,
                ValueType::Rust(output_ty),
            ));
        }
        let index_ty = self.usize_type();
        let mut result = self.graph.add_constant(0, output_ty);
        for index in 0..count {
            let bits = self
                .graph
                .oracle
                .semantic()
                .discriminant(ty, index as u32)
                .ok_or_else(|| LayoutError::Malformed("enum missing source discriminant".into()))?;
            let index = self.graph.add_constant(index as u128, index_ty);
            let condition = self.equal(variant, index);
            let value = self.graph.add_constant(bits, output_ty);
            result = self.select_scalar(condition, value, result, output_ty);
        }
        Ok(result)
    }

    pub(super) fn read_variant_scalar(
        &mut self,
        address: NodeId,
        ty: TyId,
        control: &mut NodeId,
        effect: &mut NodeId,
    ) -> Result<VariantScalar, LayoutError> {
        let layout = self
            .graph
            .oracle
            .enums
            .get(&ty)
            .cloned()
            .ok_or_else(|| LayoutError::Unsupported("enum lacks layout answer".into()))?;
        let index_ty = self.usize_type();
        if let TagEncoding::Single { variant, .. } = layout.encoding {
            return Ok(VariantScalar::Index(
                self.graph.add_constant(u128::from(variant), index_ty),
            ));
        }
        if matches!(layout.encoding, TagEncoding::Empty) {
            return Err(LayoutError::Unsupported(
                "load of an uninhabited enum".into(),
            ));
        }
        let tag = layout
            .tag
            .ok_or_else(|| LayoutError::Malformed("enum encoding lacks tag".into()))?;
        let pointer = self.field_address(address, tag.offset, tag.ty);
        let raw = self.load(pointer, tag.ty, control, effect);
        self.decode_variant_scalar(raw, ty)
    }

    pub(super) fn decode_tag_index(
        &mut self,
        raw: NodeId,
        ty: TyId,
    ) -> Result<NodeId, LayoutError> {
        let layout = self
            .graph
            .oracle
            .enums
            .get(&ty)
            .cloned()
            .ok_or_else(|| LayoutError::Unsupported("enum lacks layout answer".into()))?;
        let tag = layout
            .tag
            .ok_or_else(|| LayoutError::Malformed("enum encoding lacks tag".into()))?;
        let index_ty = self.usize_type();
        match layout.encoding {
            TagEncoding::Direct { discriminants } => {
                if discriminants
                    .iter()
                    .enumerate()
                    .all(|(index, &bits)| bits == index as u128)
                {
                    return Ok(self.graph.add_node(
                        Opcode::IntCast,
                        NodeId::NONE,
                        &[raw],
                        0,
                        ValueType::Rust(index_ty),
                    ));
                }
                let mut result = self.graph.add_constant(0, index_ty);
                for (index, bits) in discriminants.into_iter().enumerate() {
                    let bits = self.graph.add_constant(bits, tag.ty);
                    let condition = self.equal(raw, bits);
                    let value = self.graph.add_constant(index as u128, index_ty);
                    result = self.select_scalar(condition, value, result, index_ty);
                }
                Ok(result)
            }
            TagEncoding::Niche {
                untagged_variant,
                niche_variants,
                niche_start,
            } => {
                if niche_variants.start() == niche_variants.end() {
                    // One niche value: the index is a single tag equality.
                    let tag_ty = self.graph.rust_type(raw).unwrap_or(tag.ty);
                    let niche_bits = self.niche_tag_bits(tag_ty, niche_start, 0);
                    let niche = self.graph.add_constant(niche_bits, tag_ty);
                    let niche_index = u128::from(*niche_variants.start());
                    let untagged = u128::from(untagged_variant);
                    let predicate = if niche_index + 1 == untagged {
                        ComparePredicate::NotEqual
                    } else {
                        ComparePredicate::Equal
                    };
                    let boolean = self.bool_type();
                    let is_niche = self.graph.add_node(
                        Opcode::Compare,
                        NodeId::NONE,
                        &[raw, niche],
                        predicate as u32,
                        ValueType::Rust(boolean),
                    );
                    let (yes, no) = if predicate == ComparePredicate::Equal {
                        (niche_index, untagged)
                    } else {
                        (untagged, niche_index)
                    };
                    let yes = self.graph.add_constant(yes, index_ty);
                    let no = self.graph.add_constant(no, index_ty);
                    return Ok(self.select_scalar(is_niche, yes, no, index_ty));
                }
                let (relative, unsigned) = self.relative_niche_tag(raw, tag.ty, niche_start);
                let limit = self.graph.add_constant(
                    u128::from(niche_variants.end() - niche_variants.start()),
                    unsigned,
                );
                let boolean = self.bool_type();
                let in_range = self.graph.add_node(
                    Opcode::Compare,
                    NodeId::NONE,
                    &[relative, limit],
                    ComparePredicate::UnsignedLessEqual as u32,
                    ValueType::Rust(boolean),
                );
                let relative = self.graph.add_node(
                    Opcode::IntCast,
                    NodeId::NONE,
                    &[relative],
                    0,
                    ValueType::Rust(index_ty),
                );
                let start = self
                    .graph
                    .add_constant(u128::from(*niche_variants.start()), index_ty);
                let candidate = self.graph.add_node(
                    Opcode::Add,
                    NodeId::NONE,
                    &[relative, start],
                    0,
                    ValueType::Rust(index_ty),
                );
                let fallback = self
                    .graph
                    .add_constant(u128::from(untagged_variant), index_ty);
                Ok(self.select_scalar(in_range, candidate, fallback, index_ty))
            }
            _ => unreachable!(),
        }
    }

    pub(super) fn split(&mut self, control: NodeId, condition: NodeId) -> (NodeId, NodeId) {
        let branch = self.graph.add_node(
            Opcode::If,
            control,
            &[condition],
            0,
            ValueType::Tuple(vec![ValueType::Control, ValueType::Control]),
        );
        (
            self.projection(branch, 0, ValueType::Control),
            self.projection(branch, 1, ValueType::Control),
        )
    }

    pub(super) fn merge(
        &mut self,
        yes: NodeId,
        no: NodeId,
        yes_effect: NodeId,
        no_effect: NodeId,
    ) -> (NodeId, NodeId) {
        let yes = self
            .graph
            .add_node(Opcode::Jump, yes, &[], 0, ValueType::Control);
        let no = self
            .graph
            .add_node(Opcode::Jump, no, &[], 0, ValueType::Control);
        let region = self.graph.add_node(
            Opcode::Region,
            NodeId::NONE,
            &[yes, no],
            0,
            ValueType::Control,
        );
        let effect = self.graph.add_node(
            Opcode::Phi,
            NodeId::NONE,
            &[region, yes_effect, no_effect],
            0,
            ValueType::Effect,
        );
        (region, effect)
    }

    pub(super) fn decode_variant_demand(
        &mut self,
        address: NodeId,
        ty: TyId,
        control: &mut NodeId,
        effect: &mut NodeId,
        demand: &Demand,
    ) -> Result<Value, LayoutError> {
        self.decode_variant_with_transport(
            address,
            ty,
            control,
            effect,
            demand,
            &boundary_transport::Fields::new(),
        )
    }

    pub(super) fn decode_variant_with_transport(
        &mut self,
        address: NodeId,
        ty: TyId,
        control: &mut NodeId,
        effect: &mut NodeId,
        demand: &Demand,
        transport: &boundary_transport::Fields,
    ) -> Result<Value, LayoutError> {
        let variant = self.read_variant_scalar(address, ty, control, effect)?;
        let count = self
            .graph
            .oracle
            .semantic()
            .variant_count(ty)
            .ok_or_else(|| LayoutError::Malformed("enum missing semantic variants".into()))?;
        let mut variants = Vec::new();
        let plan = self.lane_plan(ty);
        let mut lanes = BTreeMap::new();
        let only_variant = match self.graph.oracle.enums[&ty].encoding {
            TagEncoding::Single { variant, .. } => Some(variant),
            _ => None,
        };
        for index in 0..count {
            if only_variant.is_some_and(|inhabited| inhabited != index as u32) {
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
                continue;
            }
            let layout_fields = self.fields(ty, Some(index as u32))?;
            if layout_fields.iter().enumerate().all(|(field, _)| {
                demand
                    .child(DemandKey::VariantField(index as u32, field as u32))
                    .is_none()
            }) {
                variants.push(
                    layout_fields
                        .iter()
                        .map(|field| self.omitted(field.ty))
                        .collect(),
                );
                continue;
            }
            if only_variant == Some(index as u32) {
                let mut fields = Vec::new();
                for (field_index, field) in layout_fields.into_iter().enumerate() {
                    let child =
                        demand.child(DemandKey::VariantField(index as u32, field_index as u32));
                    if child.is_none() {
                        fields.push(self.omitted(field.ty));
                        continue;
                    }
                    let address = self.field_address(address, field.offset, field.ty);
                    fields.push(
                        self.decode_fields_demand(address, field.ty, control, effect, &child)?,
                    );
                }
                variants.push(fields);
                continue;
            }
            // Shared lanes are read once and unconditionally, before any
            // variant guard; inactive lane bytes are only unused poison.
            let mut shared_fields = Vec::new();
            let mut guarded = false;
            for (field_index, field) in layout_fields.iter().enumerate() {
                let child = demand.child(DemandKey::VariantField(index as u32, field_index as u32));
                let shared = plan
                    .as_ref()
                    .is_some_and(|plan| plan.shares(index as u32, field_index))
                    && !transport.contains(&(index as u32, field_index as u32));
                if !shared {
                    guarded |= !child.is_none();
                    shared_fields.push(None);
                    continue;
                }
                let mut source = variant_lanes::LaneSource::Load {
                    address,
                    control: &mut *control,
                    effect: &mut *effect,
                };
                shared_fields.push(Some(self.lane_value(
                    field.ty,
                    field.offset,
                    &child,
                    &mut source,
                    &mut lanes,
                )?));
            }
            if !guarded {
                let mut fields = Vec::new();
                for (shared, field) in shared_fields.into_iter().zip(&layout_fields) {
                    fields.push(shared.unwrap_or_else(|| self.omitted(field.ty)));
                }
                variants.push(fields);
                continue;
            }
            let condition = self.variant_condition(variant, ty, index as u32)?;
            let (mut yes, no) = self.split(*control, condition);
            let original_effect = *effect;
            let mut yes_effect = original_effect;
            let mut fields = Vec::new();
            let mut lane_backed = Vec::new();
            for (field_index, field) in layout_fields.into_iter().enumerate() {
                lane_backed.push(shared_fields[field_index].is_some());
                if let Some(shared) = shared_fields[field_index].take() {
                    fields.push(shared);
                    continue;
                }
                let child = demand.child(DemandKey::VariantField(index as u32, field_index as u32));
                if child.is_none() {
                    fields.push(self.omitted(field.ty));
                    continue;
                }
                let address = self.field_address(address, field.offset, field.ty);
                fields.push(if transport.contains(&(index as u32, field_index as u32)) {
                    self.capture_owned_field(address, field.ty, &mut yes, &mut yes_effect)?
                } else {
                    self.decode_fields_demand(address, field.ty, &mut yes, &mut yes_effect, &child)?
                });
            }
            let (region, merged_effect) = self.merge(yes, no, yes_effect, original_effect);
            let mut merged = Vec::new();
            for (field_index, field) in fields.into_iter().enumerate() {
                if lane_backed[field_index] {
                    merged.push(field);
                    continue;
                }
                if let Parts::OwnedStorage(address) = field.parts {
                    merged.push(self.merge_owned_field(field, region, address));
                    continue;
                }
                let child = demand.child(DemandKey::VariantField(index as u32, field_index as u32));
                let target = self.phi_tree_demand(field.ty, region, 3, &child)?;
                let undefined = self.undefined_demand(field.ty, &child)?;
                self.fill_phi(&target, &field, 1)?;
                self.fill_phi(&target, &undefined, 2)?;
                merged.push(target);
            }
            variants.push(merged);
            *control = region;
            *effect = merged_effect;
        }
        Ok(Value {
            ty,
            parts: Parts::Variant { variant, variants },
        })
    }
}
