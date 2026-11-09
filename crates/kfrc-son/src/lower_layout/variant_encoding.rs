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
                    let tag = layout
                        .tag
                        .ok_or_else(|| LayoutError::Malformed("enum lacks tag field".into()))?;
                    Some(self.niche_tag_bits(tag.ty, niche_start, variant - niche_variants.start()))
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
            self.encode_variant_fields(value.ty, *variant, fields, None, address, control, effect)?;
            return self.write_variant_tag(address, value.ty, *variant, control, effect);
        }
        let Parts::Variant { variant, variants } = &value.parts else {
            unreachable!()
        };
        let variant = *variant;
        if let Some(index) = self.constant_variant_index(variant, value.ty)? {
            let values = variants
                .get(index as usize)
                .ok_or_else(|| LayoutError::Malformed("variant index outside enum".into()))?;
            self.encode_variant_fields(value.ty, index, values, None, address, control, effect)?;
            return self.write_variant_tag(address, value.ty, index, control, effect);
        }
        let direct = match self
            .graph
            .oracle
            .enums
            .get(&value.ty)
            .map(|layout| &layout.encoding)
        {
            Some(TagEncoding::Direct { .. }) => true,
            _ => false,
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
        // Lanes are disjoint and written first; a guarded payload field of the
        // active variant may then overwrite bytes a lane carried as poison.
        let plan = self.lane_plan(value.ty);
        if let Some(plan) = &plan {
            self.encode_lanes(value.ty, variants, plan, address, control, effect)?;
        }
        for (index, values) in variants.iter().enumerate() {
            if only_variant.is_some_and(|inhabited| inhabited != index as u32) {
                continue;
            }
            if only_variant == Some(index as u32) {
                self.encode_variant_fields(
                    value.ty,
                    index as u32,
                    values,
                    None,
                    address,
                    control,
                    effect,
                )?;
                continue;
            }
            // Direct tag storage is shared below; variants whose payload is
            // entirely in lanes need no branch or payload work at all.
            if direct
                && (0..values.len()).all(|field| {
                    plan.as_ref()
                        .is_some_and(|plan| plan.shares(index as u32, field))
                })
            {
                continue;
            }
            let condition = self.variant_condition(variant, value.ty, index as u32)?;
            let (mut yes, no) = self.split(*control, condition);
            let original_effect = *effect;
            let mut yes_effect = original_effect;
            self.encode_variant_fields(
                value.ty,
                index as u32,
                values,
                plan.as_deref(),
                address,
                &mut yes,
                &mut yes_effect,
            )?;
            if !direct {
                self.write_variant_tag(address, value.ty, index as u32, &mut yes, &mut yes_effect)?;
            }
            (*control, *effect) = self.merge(yes, no, yes_effect, original_effect);
        }
        if let VariantScalar::Direct(bits) = variant {
            let tag = self.graph.oracle.enums[&value.ty]
                .tag
                .ok_or_else(|| LayoutError::Malformed("direct enum lacks tag".into()))?;
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
        lanes: Option<&variant_lanes::LanePlan>,
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
        for (index, (field, value)) in fields.into_iter().zip(values).enumerate() {
            if lanes.is_some_and(|lanes| lanes.shares(variant, index)) {
                continue;
            }
            let address = self.field_address(address, field.offset, field.ty);
            self.encode_fields(value, address, control, effect)?;
        }
        Ok(())
    }
}
