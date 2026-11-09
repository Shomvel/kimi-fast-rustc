//! Encode active payloads and one shared direct tag.
use super::*;
use kfrc_body::TagEncoding;

impl LayoutLowering<'_> {
    pub(super) fn write_variant_tag(
        &mut self,
        address: NodeId,
        ty: TyId,
        variant: u32,
        control: &mut NodeId,
        effect: &mut NodeId,
    ) -> Result<(), LayoutError> {
        let layout = self
            .graph
            .oracle
            .enums
            .get(&ty)
            .cloned()
            .ok_or_else(|| LayoutError::Unsupported("enum lacks physical encoding".into()))?;
        let bits = match layout.encoding {
            TagEncoding::Empty => {
                return Err(LayoutError::Unsupported(
                    "uninhabited enum construction".into(),
                ));
            }
            TagEncoding::Single {
                variant: inhabited, ..
            } if inhabited == variant => None,
            TagEncoding::Single { .. } => {
                return Err(LayoutError::Unsupported(
                    "uninhabited variant construction".into(),
                ));
            }
            TagEncoding::Direct { discriminants } => {
                Some(*discriminants.get(variant as usize).ok_or_else(|| {
                    LayoutError::Malformed("missing physical discriminant".into())
                })?)
            }
            TagEncoding::Niche {
                untagged_variant,
                niche_variants,
                niche_start,
            } => {
                if variant == untagged_variant {
                    None
                } else if niche_variants.contains(&variant) {
                    Some(niche_start.wrapping_add(u128::from(variant - niche_variants.start())))
                } else {
                    return Err(LayoutError::Malformed(
                        "variant outside niche encoding".into(),
                    ));
                }
            }
        };
        if let Some(bits) = bits {
            let tag = layout
                .tag
                .ok_or_else(|| LayoutError::Malformed("enum lacks tag field".into()))?;
            let pointer = self.field_address(address, tag.offset, tag.ty);
            let value = self.graph.add_constant(bits, tag.ty);
            self.store(pointer, value, control, effect);
        }
        Ok(())
    }

    pub(super) fn encode_variant(
        &mut self,
        value: &Value,
        address: NodeId,
        control: &mut NodeId,
        effect: &mut NodeId,
    ) -> Result<(), LayoutError> {
        if let Parts::KnownVariant {
            variant, fields, ..
        } = &value.parts
        {
            self.encode_variant_fields(value.ty, *variant, fields, address, control, effect)?;
            return self.write_variant_tag(address, value.ty, *variant, control, effect);
        }
        let Parts::Variant { variant, variants } = &value.parts else {
            unreachable!()
        };
        let variant = self.graph.resolve(*variant);
        if self.graph.op(variant) == Opcode::Const {
            let index = u32::try_from(self.graph.constant_value(variant))
                .map_err(|_| LayoutError::Malformed("variant index exceeds u32".into()))?;
            let values = variants
                .get(index as usize)
                .ok_or_else(|| LayoutError::Malformed("variant index outside enum".into()))?;
            self.encode_variant_fields(value.ty, index, values, address, control, effect)?;
            return self.write_variant_tag(address, value.ty, index, control, effect);
        }
        let index_ty = self.usize_type();
        let direct = match self
            .graph
            .oracle
            .enums
            .get(&value.ty)
            .map(|layout| &layout.encoding)
        {
            Some(TagEncoding::Direct { discriminants }) => Some(discriminants.clone()),
            _ => None,
        };
        let only_variant = match self
            .graph
            .oracle
            .enums
            .get(&value.ty)
            .map(|layout| &layout.encoding)
        {
            Some(TagEncoding::Single { variant, .. }) => Some(*variant),
            _ => None,
        };
        for (index, values) in variants.iter().enumerate() {
            if only_variant.is_some_and(|inhabited| inhabited != index as u32) {
                continue;
            }
            if only_variant == Some(index as u32) {
                self.encode_variant_fields(
                    value.ty,
                    index as u32,
                    values,
                    address,
                    control,
                    effect,
                )?;
                continue;
            }
            // Direct tag storage is shared below; fieldless variants need no
            // branch or payload work at all.
            if values.is_empty() && direct.is_some() {
                continue;
            }
            let index_node = self.graph.add_constant(index as u128, index_ty);
            let condition = self.equal(variant, index_node);
            let (mut yes, no) = self.split(*control, condition);
            let original_effect = *effect;
            let mut yes_effect = original_effect;
            self.encode_variant_fields(
                value.ty,
                index as u32,
                values,
                address,
                &mut yes,
                &mut yes_effect,
            )?;
            if direct.is_none() {
                self.write_variant_tag(address, value.ty, index as u32, &mut yes, &mut yes_effect)?;
            }
            (*control, *effect) = self.merge(yes, no, yes_effect, original_effect);
        }
        if let Some(discriminants) = direct {
            let tag = self.graph.oracle.enums[&value.ty]
                .tag
                .ok_or_else(|| LayoutError::Malformed("direct enum lacks tag".into()))?;
            let bits = if discriminants
                .iter()
                .enumerate()
                .all(|(index, &bits)| bits == index as u128)
            {
                self.graph.add_node(
                    Opcode::IntCast,
                    NodeId::NONE,
                    &[variant],
                    0,
                    ValueType::Rust(tag.ty),
                )
            } else {
                let mut result = self.graph.add_constant(0, tag.ty);
                for (index, bits) in discriminants.into_iter().enumerate() {
                    let index = self.graph.add_constant(index as u128, index_ty);
                    let condition = self.equal(variant, index);
                    let bits = self.graph.add_constant(bits, tag.ty);
                    result = self.select_scalar(condition, bits, result, tag.ty);
                }
                result
            };
            let address = self.field_address(address, tag.offset, tag.ty);
            self.store(address, bits, control, effect);
        }
        Ok(())
    }

    fn encode_variant_fields(
        &mut self,
        ty: TyId,
        variant: u32,
        values: &[Value],
        address: NodeId,
        control: &mut NodeId,
        effect: &mut NodeId,
    ) -> Result<(), LayoutError> {
        let fields = self.fields(ty, Some(variant))?;
        if fields.len() != values.len() {
            return Err(LayoutError::Malformed(
                "variant field layout mismatch".into(),
            ));
        }
        for (field, value) in fields.into_iter().zip(values) {
            let address = self.field_address(address, field.offset, field.ty);
            self.encode_fields(value, address, control, effect)?;
        }
        Ok(())
    }
}
