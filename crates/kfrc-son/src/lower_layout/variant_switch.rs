//! Branch on physical tags directly, without decoding and re-comparing niches.
use super::*;
use crate::ComparePredicate;
use kfrc_body::TagEncoding;

impl LayoutLowering<'_> {
    pub(super) fn lower_variant_switch(&mut self, node: NodeId) -> Result<(), LayoutError> {
        let source = self.expand(self.graph.input(node, 0))?;
        let (logical, raw) = match source.parts {
            Parts::KnownVariant { index, .. } => (Some(index), None),
            Parts::EncodedVariant(raw) => (None, Some(raw)),
            Parts::Variant { variant, .. } | Parts::VariantTag(variant) => {
                (Some(variant), self.variant_tags.get(&variant).copied())
            }
            _ => {
                return Err(LayoutError::Malformed(
                    "VariantSwitch operand is not enum".into(),
                ));
            }
        };
        let cases = self.graph.variant_switches[self.graph.payload(node) as usize].clone();
        let mut control = self.graph.resolve(self.graph.control(node));
        for (projection, variant) in cases.iter().enumerate() {
            let condition = if let Some(raw) = raw {
                self.tag_condition(raw, source.ty, *variant)?
            } else {
                let word = self.usize_type();
                let expected = self.graph.add_constant(u128::from(*variant), word);
                self.equal(logical.unwrap(), expected)
            };
            let (yes, no) = self.split(control, condition);
            self.redirect_switch_edge(node, projection as u32, yes);
            control = no;
        }
        self.redirect_switch_edge(node, cases.len() as u32, control);
        self.graph.remove_node(node);
        Ok(())
    }

    fn redirect_switch_edge(&mut self, node: NodeId, projection: u32, control: NodeId) {
        if let Some(old) = self.projections.find(self.graph, node, projection) {
            self.graph.replace(old, control);
        } else {
            self.graph
                .add_node(Opcode::Unreachable, control, &[], 0, ValueType::Control);
        }
    }

    fn tag_condition(
        &mut self,
        raw: NodeId,
        ty: TyId,
        variant: u32,
    ) -> Result<NodeId, LayoutError> {
        let layout = self
            .graph
            .oracle
            .enums
            .get(&ty)
            .cloned()
            .ok_or_else(|| LayoutError::Unsupported("enum lacks layout answer".into()))?;
        let tag_ty = self
            .graph
            .rust_type(raw)
            .ok_or_else(|| LayoutError::Malformed("tag has no scalar type".into()))?;
        match layout.encoding {
            TagEncoding::Direct { discriminants } => {
                let bits = *discriminants.get(variant as usize).ok_or_else(|| {
                    LayoutError::Malformed("switch variant outside encoding".into())
                })?;
                let expected = self.graph.add_constant(bits, tag_ty);
                Ok(self.equal(raw, expected))
            }
            TagEncoding::Niche {
                untagged_variant,
                niche_variants,
                niche_start,
            } => {
                if niche_variants.contains(&variant) {
                    let bits =
                        niche_start.wrapping_add(u128::from(variant - niche_variants.start()));
                    let expected = self.graph.add_constant(bits, tag_ty);
                    return Ok(self.equal(raw, expected));
                }
                if variant != untagged_variant {
                    return Err(LayoutError::Malformed(
                        "switch variant outside niche encoding".into(),
                    ));
                }
                // Singleton niches (including Option<&T>) need just != tag.
                if niche_variants.start() == niche_variants.end() {
                    let expected = self.graph.add_constant(niche_start, tag_ty);
                    let boolean = self.bool_type();
                    return Ok(self.graph.add_node(
                        Opcode::Compare,
                        NodeId::NONE,
                        &[raw, expected],
                        ComparePredicate::NotEqual as u32,
                        ValueType::Rust(boolean),
                    ));
                }
                let width = match self.graph.oracle.shape(tag_ty) {
                    TypeShape::Int { width, .. } => *width,
                    _ => IntegerWidth::Pointer,
                };
                let unsigned = self.graph.oracle.intern(
                    TypeShape::Int {
                        width,
                        signed: false,
                    },
                    self.graph.oracle.size_align(tag_ty),
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
                Ok(self.graph.add_node(
                    Opcode::Compare,
                    NodeId::NONE,
                    &[relative, limit],
                    ComparePredicate::UnsignedGreater as u32,
                    ValueType::Rust(boolean),
                ))
            }
            _ => Err(LayoutError::Malformed(
                "captured tag has no tag encoding".into(),
            )),
        }
    }
}
