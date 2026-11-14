//! Logical projection and functional field updates, including partial initialization.

use super::*;

impl Builder<'_> {
    pub(in super::super) fn semantic_index(
        &self,
        ty: TyId,
        offset: u64,
        min_length: u64,
        from_end: bool,
    ) -> Result<u32, BuildError> {
        let TypeShape::Array { length, .. } = self.graph.oracle.shape(ty) else {
            return self.unsupported("constant index of a semantic non-array");
        };
        let index = if from_end {
            length.checked_sub(offset)
        } else {
            Some(offset)
        };
        if *length < min_length
            || index.is_none_or(|index| index >= *length || index > u64::from(u32::MAX))
        {
            return self.unsupported("constant array index is outside its logical bounds");
        }
        Ok(index.expect("checked above") as u32)
    }

    pub(in super::super) fn semantic_read_place(
        &mut self,
        place: Place,
        block: usize,
        control: &mut NodeId,
    ) -> Result<NodeId, BuildError> {
        if self.memory_locals[place.local.index()]
            || self
                .body
                .projection(place.projection)
                .iter()
                .any(|element| {
                    matches!(
                        element,
                        ProjectionElem::Deref
                            | ProjectionElem::Index(_)
                            | ProjectionElem::Subslice { .. }
                    )
                })
        {
            let address = self.address(place, block, control)?;
            if self.graph.oracle.semantic().is_unsized(address.ty) {
                return self.unsupported("cannot load an unsized place as a whole value");
            }
            return Ok(self.load(address.address, address.ty, block, control));
        }
        let mut ty = self.body.locals[place.local.index()].ty;
        let mut value = self.read_variable(place.local.0, block);
        let mut variant = None;
        for element in self.body.projection(place.projection).to_vec() {
            match element {
                ProjectionElem::Downcast(index) => {
                    if !self.graph.oracle.semantic().is_enum(ty) {
                        return self.unsupported("downcast of a semantic non-enum");
                    }
                    variant = Some(index.0);
                }
                ProjectionElem::Field(index, expected) => {
                    if self
                        .graph
                        .oracle
                        .semantic()
                        .maybe_uninit_inner(ty)
                        .is_some()
                    {
                        if variant.is_some()
                            || self.graph.oracle.semantic().field_type(ty, None, index.0)
                                != Some(expected)
                        {
                            return self.unsupported("MaybeUninit field projection type mismatch");
                        }
                        value = self.maybe_uninit_field(value, expected, block, control)?;
                    } else {
                        value = self.semantic_field(value, ty, variant, index.0)?;
                        if variant.is_some() {
                            self.variant_witness_candidates.push((value, *control));
                        }
                    }
                    if self.graph.rust_type(value) != Some(expected) {
                        return self
                            .unsupported("projected field disagrees with semantic field type");
                    }
                    ty = expected;
                    variant = None;
                }
                ProjectionElem::ConstantIndex {
                    offset,
                    min_length,
                    from_end,
                } => {
                    let index = self.semantic_index(ty, offset, min_length, from_end)?;
                    value = self.semantic_field(value, ty, None, index)?;
                    ty = self
                        .graph
                        .rust_type(value)
                        .expect("field has a Rust value type");
                    variant = None;
                }
                ProjectionElem::Deref => {
                    return self
                        .unsupported("semantic dereference requires explicit memory lowering");
                }
                ProjectionElem::Index(_) => {
                    return self.unsupported("dynamic semantic indexing is not implemented");
                }
                ProjectionElem::Subslice { .. } => {
                    return self.unsupported("semantic subslice is not implemented");
                }
            }
        }
        if variant.is_some() {
            return self.unsupported("a downcast must be followed by a field projection");
        }
        Ok(value)
    }

    pub(in super::super) fn semantic_write_place(
        &mut self,
        place: Place,
        value: NodeId,
        block: usize,
        control: &mut NodeId,
    ) -> Result<(), BuildError> {
        if self.graph.rust_type(value) != Some(self.place_type(place)?) {
            return self.unsupported("assigned value disagrees with the semantic place type");
        }
        if self.memory_locals[place.local.index()]
            || self
                .body
                .projection(place.projection)
                .iter()
                .any(|element| {
                    matches!(
                        element,
                        ProjectionElem::Deref
                            | ProjectionElem::Index(_)
                            | ProjectionElem::Subslice { .. }
                    )
                })
        {
            let address = self.address(place, block, control)?;
            if self.graph.oracle.semantic().is_unsized(address.ty) {
                return self.unsupported("cannot store an unsized place as a whole value");
            }
            self.store(address.address, value, block, control);
            return Ok(());
        }
        let elements = self.body.projection(place.projection).to_vec();
        if elements.is_empty() {
            self.write_variable(place.local.0, block, value);
            return Ok(());
        }
        let ty = self.body.locals[place.local.index()].ty;
        let old = self.read_variable(place.local.0, block);
        let updated = self.semantic_update(old, ty, &elements, value, block, control)?;
        self.write_variable(place.local.0, block, updated);
        Ok(())
    }

    fn semantic_update(
        &mut self,
        old: NodeId,
        ty: TyId,
        elements: &[ProjectionElem],
        value: NodeId,
        block: usize,
        control: &mut NodeId,
    ) -> Result<NodeId, BuildError> {
        let Some((head, tail)) = elements.split_first() else {
            return Ok(value);
        };
        if let ProjectionElem::Downcast(variant) = head {
            let Some((ProjectionElem::Field(index, expected), tail)) = tail.split_first() else {
                return self.unsupported("enum update requires a field after its downcast");
            };
            let fields = self.semantic_fields(ty, Some(variant.0))?;
            if fields.get(index.0 as usize) != Some(expected) {
                return self.unsupported("enum update disagrees with its semantic field type");
            }
            let mut values = Vec::with_capacity(fields.len());
            for (field, field_ty) in fields.into_iter().enumerate() {
                let projected = self.semantic_field(old, ty, Some(variant.0), field as u32)?;
                self.variant_witness_candidates.push((projected, *control));
                values.push(if field == index.0 as usize {
                    self.semantic_update(projected, field_ty, tail, value, block, control)?
                } else {
                    projected
                });
            }
            return Ok(self.graph.add_node(
                Opcode::MakeVariant,
                NodeId::NONE,
                &values,
                variant.0,
                ValueType::Rust(ty),
            ));
        }
        let (index, expected) = match head {
            ProjectionElem::Field(index, field_ty) => (index.0, *field_ty),
            ProjectionElem::ConstantIndex {
                offset,
                min_length,
                from_end,
            } => {
                let index = self.semantic_index(ty, *offset, *min_length, *from_end)?;
                let field_ty = self
                    .graph
                    .oracle
                    .semantic()
                    .field_type(ty, None, index)
                    .expect("checked array index");
                (index, field_ty)
            }
            ProjectionElem::Downcast(_) => unreachable!("handled above"),
            ProjectionElem::Deref => {
                return self.unsupported(
                    "semantic dereference assignment requires explicit memory lowering",
                );
            }
            ProjectionElem::Index(_) => {
                return self.unsupported("dynamic semantic indexing is not implemented");
            }
            ProjectionElem::Subslice { .. } => {
                return self.unsupported("semantic subslice assignment is not implemented");
            }
        };
        if self.graph.oracle.semantic().field_type(ty, None, index) != Some(expected) {
            return self.unsupported("assigned field disagrees with semantic field type");
        }
        if self
            .graph
            .oracle
            .semantic()
            .maybe_uninit_inner(ty)
            .is_some()
        {
            let updated = if tail.is_empty() {
                value
            } else {
                let field = self.maybe_uninit_field(old, expected, block, control)?;
                self.semantic_update(field, expected, tail, value, block, control)?
            };
            return self.maybe_uninit_pack(updated, ty, expected, block, control);
        }
        let updated = if tail.is_empty() {
            value
        } else {
            let field = self.semantic_field(old, ty, None, index)?;
            self.semantic_update(field, expected, tail, value, block, control)?
        };
        Ok(self.graph.add_node(
            Opcode::InsertField,
            NodeId::NONE,
            &[old, updated],
            index,
            ValueType::Rust(ty),
        ))
    }
}
