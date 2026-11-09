//! Explicit representation conversions; normal values never use raw addresses.
use super::*;
use kfrc_body::TagEncoding;

impl LayoutLowering<'_> {
    pub(super) fn repr_value(
        &mut self,
        ty: TyId,
        bits: u128,
        offset: u64,
    ) -> Result<Value, LayoutError> {
        if self
            .graph
            .oracle
            .semantic()
            .maybe_uninit_inner(ty)
            .is_some()
        {
            return self.opaque_constant(ty, bits, offset);
        }
        if self.graph.oracle.semantic().is_enum(ty) {
            let layout =
                self.graph.oracle.enums.get(&ty).cloned().ok_or_else(|| {
                    LayoutError::Unsupported("constant enum has no encoding".into())
                })?;
            let tag = if let Some(tag) = layout.tag {
                let size = self
                    .graph
                    .oracle
                    .size_align(tag.ty)
                    .ok_or_else(|| {
                        LayoutError::Unsupported("constant enum tag has no layout".into())
                    })?
                    .size;
                self.extract_bits(bits, offset + tag.offset, size)?
            } else {
                0
            };
            let active = match layout.encoding {
                TagEncoding::Empty => {
                    return Err(LayoutError::Unsupported("uninhabited constant enum".into()));
                }
                TagEncoding::Single { variant, .. } => variant,
                TagEncoding::Direct { discriminants } => {
                    let size = self
                        .graph
                        .oracle
                        .size_align(layout.tag.unwrap().ty)
                        .unwrap()
                        .size;
                    let mask = if size == 16 {
                        u128::MAX
                    } else {
                        (1u128 << (size * 8)) - 1
                    };
                    discriminants
                        .iter()
                        .position(|value| (*value & mask) == tag)
                        .ok_or_else(|| {
                            LayoutError::Malformed("constant enum has invalid tag".into())
                        })? as u32
                }
                TagEncoding::Niche {
                    untagged_variant,
                    niche_variants,
                    niche_start,
                } => {
                    let tag_size = self
                        .graph
                        .oracle
                        .size_align(layout.tag.unwrap().ty)
                        .unwrap()
                        .size;
                    let mask = if tag_size == 16 {
                        u128::MAX
                    } else {
                        (1u128 << (tag_size * 8)) - 1
                    };
                    let relative = tag.wrapping_sub(niche_start) & mask;
                    if relative <= u128::from(niche_variants.end() - niche_variants.start()) {
                        *niche_variants.start() + relative as u32
                    } else {
                        untagged_variant
                    }
                }
            };
            let index_ty = self.usize_type();
            let index = self.graph.add_constant(u128::from(active), index_ty);
            let fields = self
                .fields(ty, Some(active))?
                .into_iter()
                .map(|field| self.repr_value(field.ty, bits, offset + field.offset))
                .collect::<Result<_, _>>()?;
            return Ok(Value {
                ty,
                parts: Parts::KnownVariant {
                    variant: active,
                    index,
                    fields,
                },
            });
        }
        let parts = if self.composite(ty) {
            Parts::Fields(
                self.fields(ty, None)?
                    .into_iter()
                    .map(|field| self.repr_value(field.ty, bits, offset + field.offset))
                    .collect::<Result<_, _>>()?,
            )
        } else {
            let size = self
                .graph
                .oracle
                .size_align(ty)
                .ok_or_else(|| LayoutError::Unsupported("constant field has no layout".into()))?
                .size;
            Parts::Scalar(
                self.graph
                    .add_constant(self.extract_bits(bits, offset, size)?, ty),
            )
        };
        Ok(Value { ty, parts })
    }

    fn extract_bits(&self, bits: u128, offset: u64, size: u64) -> Result<u128, LayoutError> {
        if size == 0 {
            return Ok(0);
        }
        if size > 16 || offset.checked_add(size).is_none_or(|end| end > 16) {
            return Err(LayoutError::Unsupported(
                "representation constant exceeds 128 bits".into(),
            ));
        }
        let mask = if size == 16 {
            u128::MAX
        } else {
            (1u128 << (size * 8)) - 1
        };
        Ok((bits >> (offset * 8)) & mask)
    }

    pub(super) fn lower_transmute(&mut self, node: NodeId) -> Result<(), LayoutError> {
        if self.graph.is_dead(node) {
            return Ok(());
        }
        let source = self.expand(self.graph.input(node, 1))?;
        let target_ty = match self.graph.value_type(node) {
            ValueType::Tuple(elements) => match elements.get(2) {
                Some(ValueType::Rust(ty)) => *ty,
                _ => {
                    return Err(LayoutError::Malformed(
                        "transmute lacks target value type".into(),
                    ));
                }
            },
            _ => {
                return Err(LayoutError::Malformed(
                    "transmute lacks result tuple".into(),
                ));
            }
        };
        let source_layout = self
            .graph
            .oracle
            .size_align(source.ty)
            .ok_or_else(|| LayoutError::Unsupported("unsized transmute source".into()))?;
        let target_layout = self
            .graph
            .oracle
            .size_align(target_ty)
            .ok_or_else(|| LayoutError::Unsupported("unsized transmute target".into()))?;
        if source_layout.size != target_layout.size {
            return Err(LayoutError::Malformed("transmute sizes differ".into()));
        }
        let slot = self.graph.stack_slots.len() as u32;
        self.graph.stack_slots.push(SizeAlign {
            size: source_layout.size,
            align: source_layout.align.max(target_layout.align),
        });
        let pointer = self.pointer_type(source.ty);
        let address = self.graph.add_node(
            Opcode::StackSlot,
            NodeId::NONE,
            &[],
            slot,
            ValueType::Rust(pointer),
        );
        let mut control = self.graph.resolve(self.graph.control(node));
        let mut effect = self.graph.resolve(self.graph.input(node, 0));
        self.encode_fields(&source, address, &mut control, &mut effect)?;
        let target_address = self.field_address(address, 0, target_ty);
        let value = self.decode_fields(target_address, target_ty, &mut control, &mut effect)?;
        // Source validity/definedness obligations are interpreted in S. The
        // publishing seam retains actual representation reads, writes and raw
        // MaybeUninit initialization masks, but emits no diagnostic scan.
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
}
