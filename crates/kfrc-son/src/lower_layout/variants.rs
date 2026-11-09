//! Enum values keep a logical discriminant and guarded payload SSA values.
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

    pub(super) fn source_discriminant(
        &mut self,
        ty: TyId,
        variant: NodeId,
        output_ty: TyId,
    ) -> Result<NodeId, LayoutError> {
        let count = self
            .graph
            .oracle
            .semantic()
            .variant_count(ty)
            .ok_or_else(|| LayoutError::Malformed("enum has no variants".into()))?;
        let variant = self.graph.resolve(variant);
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

    pub(super) fn read_variant_index(
        &mut self,
        address: NodeId,
        ty: TyId,
        control: &mut NodeId,
        effect: &mut NodeId,
    ) -> Result<NodeId, LayoutError> {
        let layout = self
            .graph
            .oracle
            .enums
            .get(&ty)
            .cloned()
            .ok_or_else(|| LayoutError::Unsupported("enum lacks layout answer".into()))?;
        let index_ty = self.usize_type();
        if let TagEncoding::Single { variant, .. } = layout.encoding {
            return Ok(self.graph.add_constant(u128::from(variant), index_ty));
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
        let index = self.decode_tag_index(raw, ty)?;
        self.variant_tags.insert(index, raw);
        Ok(index)
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
                let width = match self.graph.oracle.shape(tag.ty) {
                    TypeShape::Int { width, .. } => *width,
                    _ => IntegerWidth::Pointer,
                };
                let unsigned = self.graph.oracle.intern(
                    TypeShape::Int {
                        width,
                        signed: false,
                    },
                    self.graph.oracle.size_align(tag.ty),
                );
                let raw = self.graph.add_node(
                    Opcode::IntCast,
                    NodeId::NONE,
                    &[raw],
                    0,
                    ValueType::Rust(unsigned),
                );
                let start = self.graph.add_constant(niche_start, unsigned);
                let relative = self.graph.add_node(
                    Opcode::Sub,
                    NodeId::NONE,
                    &[raw, start],
                    0,
                    ValueType::Rust(unsigned),
                );
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

    pub(super) fn decode_variant(
        &mut self,
        address: NodeId,
        ty: TyId,
        control: &mut NodeId,
        effect: &mut NodeId,
    ) -> Result<Value, LayoutError> {
        let variant = self.read_variant_index(address, ty, control, effect)?;
        let count = self
            .graph
            .oracle
            .semantic()
            .variant_count(ty)
            .ok_or_else(|| LayoutError::Malformed("enum missing semantic variants".into()))?;
        let mut variants = Vec::new();
        let index_ty = self.usize_type();
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
                        .map(|field| self.undefined(field))
                        .collect::<Result<_, _>>()?,
                );
                continue;
            }
            let layout_fields = self.fields(ty, Some(index as u32))?;
            if layout_fields.is_empty() {
                variants.push(Vec::new());
                continue;
            }
            if only_variant == Some(index as u32) {
                let mut fields = Vec::new();
                for field in layout_fields {
                    let address = self.field_address(address, field.offset, field.ty);
                    fields.push(self.decode_fields(address, field.ty, control, effect)?);
                }
                variants.push(fields);
                continue;
            }
            let index_node = self.graph.add_constant(index as u128, index_ty);
            let condition = self.equal(variant, index_node);
            let (mut yes, no) = self.split(*control, condition);
            let original_effect = *effect;
            let mut yes_effect = original_effect;
            let mut fields = Vec::new();
            for field in layout_fields {
                let address = self.field_address(address, field.offset, field.ty);
                fields.push(self.decode_fields(address, field.ty, &mut yes, &mut yes_effect)?);
            }
            let (region, merged_effect) = self.merge(yes, no, yes_effect, original_effect);
            let mut merged = Vec::new();
            for field in fields {
                let target = self.phi_tree(field.ty, region, 3)?;
                let undefined = self.undefined(field.ty)?;
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
