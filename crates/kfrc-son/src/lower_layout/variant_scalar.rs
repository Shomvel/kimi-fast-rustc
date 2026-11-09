//! One scalar domain per enum layout, shared by constructors, loads and Phis.
use super::*;
use kfrc_body::TagEncoding;

impl LayoutLowering<'_> {
    pub(super) fn variant_scalar_type(&mut self, ty: TyId) -> Result<TyId, LayoutError> {
        if let Some(layout) = self.graph.oracle.enums.get(&ty)
            && matches!(layout.encoding, TagEncoding::Direct { .. })
        {
            return layout
                .tag
                .map(|tag| tag.ty)
                .ok_or_else(|| LayoutError::Malformed("direct enum lacks tag".into()));
        }
        Ok(self.usize_type())
    }

    pub(super) fn variant_scalar(&self, ty: TyId, node: NodeId) -> VariantScalar {
        if self
            .graph
            .oracle
            .enums
            .get(&ty)
            .is_some_and(|layout| matches!(layout.encoding, TagEncoding::Direct { .. }))
        {
            VariantScalar::Direct(node)
        } else {
            VariantScalar::Index(node)
        }
    }

    pub(super) fn known_variant_scalar(
        &mut self,
        ty: TyId,
        index: u32,
    ) -> Result<VariantScalar, LayoutError> {
        let scalar_ty = self.variant_scalar_type(ty)?;
        let bits = match self
            .graph
            .oracle
            .enums
            .get(&ty)
            .map(|layout| &layout.encoding)
        {
            Some(TagEncoding::Direct { discriminants }) => *discriminants
                .get(index as usize)
                .ok_or_else(|| LayoutError::Malformed("variant outside direct encoding".into()))?,
            _ => u128::from(index),
        };
        let node = self.graph.add_constant(bits, scalar_ty);
        Ok(self.variant_scalar(ty, node))
    }

    pub(super) fn decode_variant_scalar(
        &mut self,
        raw: NodeId,
        ty: TyId,
    ) -> Result<VariantScalar, LayoutError> {
        match self.variant_scalar(ty, raw) {
            direct @ VariantScalar::Direct(_) => Ok(direct),
            VariantScalar::Index(_) => {
                let index = self.decode_tag_index(raw, ty)?;
                self.variant_tags.insert(index, raw);
                Ok(VariantScalar::Index(index))
            }
        }
    }

    pub(super) fn variant_condition(
        &mut self,
        scalar: VariantScalar,
        ty: TyId,
        index: u32,
    ) -> Result<NodeId, LayoutError> {
        // A decoded niche index still has its physical tag: compare the tag
        // against the variant's legal encoding instead of re-decoding.
        if let VariantScalar::Index(decoded) = scalar
            && let Some(raw) = self.variant_tags.get(&decoded).copied()
        {
            self.stats.niche_tag_conditions += 1;
            return self.tag_condition(raw, ty, index);
        }
        let expected = self.known_variant_scalar(ty, index)?;
        Ok(self.equal(scalar.node(), expected.node()))
    }

    /// Convert only for an explicit semantic VariantOf query. Discriminant,
    /// payload guards, transport and switches do not need this mapping.
    pub(super) fn variant_index(
        &mut self,
        scalar: VariantScalar,
        ty: TyId,
    ) -> Result<NodeId, LayoutError> {
        if let Some(index) = self.constant_variant_index(scalar, ty)? {
            let word = self.usize_type();
            return Ok(self.graph.add_constant(u128::from(index), word));
        }
        match scalar {
            VariantScalar::Direct(raw) => self.decode_tag_index(raw, ty),
            VariantScalar::Index(index) => Ok(index),
        }
    }

    pub(super) fn constant_variant_index(
        &self,
        scalar: VariantScalar,
        ty: TyId,
    ) -> Result<Option<u32>, LayoutError> {
        let node = self.graph.resolve(scalar.node());
        if self.graph.op(node) != Opcode::Const {
            return Ok(None);
        }
        let bits = self.graph.constant_value(node);
        let index = match scalar {
            VariantScalar::Index(_) => u32::try_from(bits).ok(),
            VariantScalar::Direct(_) => {
                let layout = &self.graph.oracle.enums[&ty];
                let TagEncoding::Direct { discriminants } = &layout.encoding else {
                    unreachable!()
                };
                let tag = layout
                    .tag
                    .ok_or_else(|| LayoutError::Malformed("direct enum lacks tag".into()))?;
                let size = self
                    .graph
                    .oracle
                    .size_align(tag.ty)
                    .ok_or_else(|| LayoutError::Malformed("direct tag lacks size".into()))?
                    .size;
                let mask = u128::MAX >> (128 - size * 8);
                discriminants
                    .iter()
                    .position(|value| value & mask == bits & mask)
                    .and_then(|index| u32::try_from(index).ok())
            }
        };
        index
            .map(Some)
            .ok_or_else(|| LayoutError::Malformed("constant variant outside encoding".into()))
    }
}
