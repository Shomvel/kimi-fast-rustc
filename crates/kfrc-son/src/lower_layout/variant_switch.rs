//! Branch on physical tags directly, without decoding and re-comparing niches.
use super::*;
use crate::ComparePredicate;
use kfrc_body::TagEncoding;

impl LayoutLowering<'_> {
    pub(super) fn lower_variant_switch(&mut self, node: NodeId) -> Result<(), LayoutError> {
        let source = self.expand(self.graph.input(node, 0))?;
        let (logical, raw) = match source.parts {
            Parts::KnownVariant { scalar, .. } => (Some(scalar), None),
            Parts::EncodedVariant(raw) => (None, Some(raw)),
            Parts::Variant { variant, .. } | Parts::VariantTag(variant) => (
                Some(variant),
                self.variant_tags.get(&variant.node()).copied(),
            ),
            _ => {
                return Err(LayoutError::Malformed(
                    "VariantSwitch operand is not enum".into(),
                ));
            }
        };
        let cases = self.graph.variant_switches[self.graph.payload(node) as usize].clone();
        let mut control = self.graph.resolve(self.graph.control(node));
        // An exhaustive switch whose remaining edge is unreachable selects
        // its last case without comparing: a valid tag matched an earlier
        // case or is this one.
        let otherwise = self.projections.find(self.graph, node, cases.len() as u32);
        let unreachable = match otherwise {
            Some(edge) => self.unreachable_switch_edges.get(&edge).copied(),
            None => None,
        };
        let all_cases_explicit = otherwise.is_none() || unreachable.is_some();
        for (projection, variant) in cases.iter().enumerate() {
            if all_cases_explicit && projection + 1 == cases.len() {
                self.redirect_switch_edge(node, projection as u32, control);
                self.stats.exhaustive_switch_compares_removed += 1;
                if let (Some(edge), Some(unreachable)) = (otherwise, unreachable) {
                    self.graph.remove_node(unreachable);
                    self.graph.remove_node(edge);
                }
                self.graph.remove_node(node);
                return Ok(());
            }
            let condition = if let Some(raw) = raw {
                self.tag_condition(raw, source.ty, *variant)?
            } else {
                self.variant_condition(logical.unwrap(), source.ty, *variant)?
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

    pub(super) fn tag_condition(
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
                // rustc's niche range may contain the untagged variant. Its
                // relative slot is never a valid tag (rustc assumes it away),
                // so only niche variants other than the untagged one compare
                // against an encoded value.
                if variant != untagged_variant && niche_variants.contains(&variant) {
                    let bits =
                        self.niche_tag_bits(tag_ty, niche_start, variant - niche_variants.start());
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
                    let bits = self.niche_tag_bits(tag_ty, niche_start, 0);
                    let expected = self.graph.add_constant(bits, tag_ty);
                    let boolean = self.bool_type();
                    return Ok(self.graph.add_node(
                        Opcode::Compare,
                        NodeId::NONE,
                        &[raw, expected],
                        ComparePredicate::NotEqual as u32,
                        ValueType::Rust(boolean),
                    ));
                }
                let (relative, unsigned) = self.relative_niche_tag(raw, tag_ty, niche_start);
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

    /// `raw - niche_start` in the tag's unsigned width; a zero start is the
    /// raw tag itself.
    pub(super) fn relative_niche_tag(
        &mut self,
        raw: NodeId,
        tag_ty: TyId,
        niche_start: u128,
    ) -> (NodeId, TyId) {
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
        let niche_start = self.niche_tag_bits(tag_ty, niche_start, 0);
        if niche_start == 0 {
            return (raw, unsigned);
        }
        let start = self.graph.add_constant(niche_start, unsigned);
        let relative = self.graph.add_node(
            Opcode::Sub,
            NodeId::NONE,
            &[raw, start],
            0,
            ValueType::Rust(unsigned),
        );
        (relative, unsigned)
    }

    /// The tag bits of niche slot `relative`, wrapped to the tag's width as
    /// rustc's niche arithmetic is.
    pub(super) fn niche_tag_bits(&self, tag_ty: TyId, niche_start: u128, relative: u32) -> u128 {
        let bits = niche_start.wrapping_add(u128::from(relative));
        match self
            .graph
            .oracle
            .size_align(tag_ty)
            .map(|layout| layout.size)
        {
            Some(size @ 1..16) => bits & ((1u128 << (size * 8)) - 1),
            _ => bits,
        }
    }
}
