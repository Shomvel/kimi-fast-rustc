//! Structured addresses and effect-ordered Rust-value memory operations.
//!
//! No field offset, aggregate byte size, or enum encoding is selected here.

use super::*;

pub(super) struct PlaceAddress {
    pub address: NodeId,
    pub ty: TyId,
    pub metadata: Option<NodeId>,
}

impl Builder<'_> {
    pub(super) fn pointer_type(&mut self, pointee: TyId) -> TyId {
        // An address keeps its logical owner, including DSTs, without being a
        // Rust pointer value. Wide pointer metadata travels beside it.
        self.graph
            .oracle
            .intern(TypeShape::PlaceAddress { pointee }, None)
    }

    pub(super) fn usize_type(&mut self) -> TyId {
        self.graph.oracle.intern(
            TypeShape::Int {
                width: IntegerWidth::Pointer,
                signed: false,
            },
            Some(SizeAlign { size: 8, align: 8 }),
        )
    }

    pub(super) fn pointer_cast(&mut self, address: NodeId, ty: TyId) -> NodeId {
        if self.graph.rust_type(address) == Some(ty) {
            return address;
        }
        self.graph.add_node(
            Opcode::PointerCast,
            NodeId::NONE,
            &[address],
            0,
            ValueType::Rust(ty),
        )
    }

    pub(super) fn load(
        &mut self,
        address: NodeId,
        ty: TyId,
        block: usize,
        control: &mut NodeId,
    ) -> NodeId {
        let effect = self.read_variable(self.effect, block);
        let load = self.graph.add_node(
            Opcode::LoadValue,
            *control,
            &[effect, address],
            0,
            ValueType::Tuple(vec![
                ValueType::Control,
                ValueType::Effect,
                ValueType::Rust(ty),
            ]),
        );
        *control = self.projection(load, 0, ValueType::Control);
        let effect = self.projection(load, 1, ValueType::Effect);
        self.write_variable(self.effect, block, effect);
        self.projection(load, 2, ValueType::Rust(ty))
    }

    pub(super) fn store(
        &mut self,
        address: NodeId,
        value: NodeId,
        block: usize,
        control: &mut NodeId,
    ) {
        let effect = self.read_variable(self.effect, block);
        let store = self.graph.add_node(
            Opcode::StoreValue,
            *control,
            &[effect, address, value],
            0,
            ValueType::Tuple(vec![ValueType::Control, ValueType::Effect]),
        );
        *control = self.projection(store, 0, ValueType::Control);
        let effect = self.projection(store, 1, ValueType::Effect);
        self.write_variable(self.effect, block, effect);
    }

    pub(super) fn place_type(&self, place: Place) -> Result<TyId, BuildError> {
        let mut ty = self.body.locals[place.local.index()].ty;
        for element in self.body.projection(place.projection) {
            ty = match element {
                ProjectionElem::Field(_, ty) => *ty,
                ProjectionElem::Deref => match self.body.types.shape(ty) {
                    TypeShape::Reference { pointee, .. }
                    | TypeShape::RawPointer { pointee, .. } => *pointee,
                    _ => return self.unsupported("dereference of non-pointer"),
                },
                ProjectionElem::Index(_) | ProjectionElem::ConstantIndex { .. } => {
                    match self.body.types.shape(ty) {
                        TypeShape::Array { element, .. } | TypeShape::Slice { element } => *element,
                        _ => return self.unsupported("index of non-array"),
                    }
                }
                ProjectionElem::Downcast(_) => ty,
                ProjectionElem::Subslice { result_ty, .. } => *result_ty,
            };
        }
        Ok(ty)
    }

    pub(super) fn address(
        &mut self,
        place: Place,
        block: usize,
        control: &mut NodeId,
    ) -> Result<PlaceAddress, BuildError> {
        let mut ty = self.body.locals[place.local.index()].ty;
        let mut address = (!self.local_slots[place.local.index()].is_none())
            .then_some(self.local_slots[place.local.index()]);
        let mut value = None;
        let mut metadata = None;
        let mut variant = None;
        for projection in self.body.projection(place.projection).to_vec() {
            match projection {
                ProjectionElem::Deref => {
                    let pointer = if let Some(address) = address {
                        self.load(address, ty, block, control)
                    } else {
                        value.unwrap_or_else(|| self.read_variable(place.local.0, block))
                    };
                    let pointee = match self.graph.oracle.shape(ty) {
                        TypeShape::Reference { pointee, .. }
                        | TypeShape::RawPointer { pointee, .. } => *pointee,
                        _ => return self.unsupported("dereference of non-pointer"),
                    };
                    let data = if self.semantic_scalar(ty) {
                        metadata = None;
                        pointer
                    } else {
                        metadata = Some(self.semantic_field(pointer, ty, None, 1)?);
                        self.semantic_field(pointer, ty, None, 0)?
                    };
                    let pointer_ty = self.pointer_type(pointee);
                    address = Some(self.pointer_cast(data, pointer_ty));
                    value = None;
                    ty = pointee;
                    variant = None;
                }
                ProjectionElem::Downcast(index) => {
                    variant = Some(index.0);
                }
                ProjectionElem::Field(index, field_ty) => {
                    if self.graph.oracle.semantic().is_unsized(field_ty) {
                        if variant.is_some()
                            || self.graph.oracle.semantic().static_slice_tail(ty) != Some(index.0)
                        {
                            return self.unsupported(
                                "unsized field requires a producer-proven static slice/string tail",
                            );
                        }
                        if metadata.is_none() {
                            return self.unsupported("unsized field address has no metadata");
                        }
                    } else {
                        metadata = None;
                    }
                    if self
                        .graph
                        .oracle
                        .semantic()
                        .field_type(ty, variant, index.0)
                        != Some(field_ty)
                    {
                        return self
                            .unsupported("address projection disagrees with semantic field type");
                    }
                    if let Some(base) = address {
                        let pointer_ty = self.pointer_type(field_ty);
                        let (opcode, payload) = if let Some(variant) = variant {
                            (
                                Opcode::VariantFieldAddr,
                                self.graph.intern_variant_field(variant, index.0),
                            )
                        } else {
                            (Opcode::FieldAddr, index.0)
                        };
                        address = Some(self.graph.add_node(
                            opcode,
                            NodeId::NONE,
                            &[base],
                            payload,
                            ValueType::Rust(pointer_ty),
                        ));
                    } else {
                        let parent =
                            value.unwrap_or_else(|| self.read_variable(place.local.0, block));
                        value = Some(self.semantic_field(parent, ty, variant, index.0)?);
                    }
                    ty = field_ty;
                    variant = None;
                }
                ProjectionElem::Index(index_local) => {
                    let index = self.read_place(Place::local(index_local), block, control)?;
                    let element = self.index_element(ty)?;
                    let base = address.ok_or_else(|| BuildError::Unsupported {
                        block: self.current_block,
                        what: "dynamic indexing requires a materialized local".into(),
                    })?;
                    address = Some(self.index_address(base, index, element));
                    ty = element;
                    metadata = None;
                    variant = None;
                }
                ProjectionElem::ConstantIndex {
                    offset,
                    min_length,
                    from_end,
                } => {
                    let element = self.index_element(ty)?;
                    let usize_ty = self.usize_type();
                    let constant = self.graph.add_constant(u128::from(offset), usize_ty);
                    let index = if from_end {
                        let length = self.place_length(ty, metadata)?;
                        self.graph.add_node(
                            Opcode::Sub,
                            NodeId::NONE,
                            &[length, constant],
                            0,
                            ValueType::Rust(usize_ty),
                        )
                    } else {
                        constant
                    };
                    if let TypeShape::Array { length, .. } = self.graph.oracle.shape(ty) {
                        let index = if from_end {
                            length.checked_sub(offset)
                        } else {
                            Some(offset)
                        };
                        if *length < min_length || index.is_none_or(|index| index >= *length) {
                            return self.unsupported("array index outside its logical bounds");
                        }
                    }
                    if let Some(base) = address {
                        address = Some(self.index_address(base, index, element));
                    } else {
                        let parent =
                            value.unwrap_or_else(|| self.read_variable(place.local.0, block));
                        let index = self.semantic_index(ty, offset, min_length, from_end)?;
                        value = Some(self.semantic_field(parent, ty, None, index)?);
                    }
                    ty = element;
                    metadata = None;
                    variant = None;
                }
                ProjectionElem::Subslice {
                    from,
                    to,
                    from_end,
                    result_ty,
                } => {
                    let element = self.index_element(ty)?;
                    let usize_ty = self.usize_type();
                    let start = self.graph.add_constant(u128::from(from), usize_ty);
                    let base = address.ok_or_else(|| BuildError::Unsupported {
                        block: self.current_block,
                        what: "subslice requires addressable storage".into(),
                    })?;
                    let data = self.index_address(base, start, element);
                    let pointer_ty = self.pointer_type(result_ty);
                    address = Some(self.pointer_cast(data, pointer_ty));
                    let length = if from_end {
                        let length = self.place_length(ty, metadata)?;
                        let removed = self
                            .graph
                            .add_constant(u128::from(from) + u128::from(to), usize_ty);
                        self.graph.add_node(
                            Opcode::Sub,
                            NodeId::NONE,
                            &[length, removed],
                            0,
                            ValueType::Rust(usize_ty),
                        )
                    } else {
                        self.graph.add_constant(
                            u128::from(to.checked_sub(from).ok_or_else(|| {
                                BuildError::Unsupported {
                                    block: self.current_block,
                                    what: "subslice end before start".into(),
                                }
                            })?),
                            usize_ty,
                        )
                    };
                    metadata = self
                        .graph
                        .oracle
                        .semantic()
                        .is_unsized(result_ty)
                        .then_some(length);
                    ty = result_ty;
                    variant = None;
                }
            }
        }
        let address = address.ok_or_else(|| BuildError::Unsupported {
            block: self.current_block,
            what: "place has no materialized storage".into(),
        })?;
        Ok(PlaceAddress {
            address,
            ty,
            metadata,
        })
    }

    fn index_element(&mut self, ty: TyId) -> Result<TyId, BuildError> {
        match self.graph.oracle.shape(ty) {
            TypeShape::Array { element, .. } | TypeShape::Slice { element } => Ok(*element),
            TypeShape::Str => Ok(self.graph.oracle.intern(
                TypeShape::Int {
                    width: IntegerWidth::W8,
                    signed: false,
                },
                Some(SizeAlign { size: 1, align: 1 }),
            )),
            _ => self.unsupported("index of non-array/slice"),
        }
    }

    pub(super) fn index_address(&mut self, base: NodeId, index: NodeId, element: TyId) -> NodeId {
        let pointer_ty = self.pointer_type(element);
        self.graph.add_node(
            Opcode::IndexAddr,
            NodeId::NONE,
            &[base, index],
            0,
            ValueType::Rust(pointer_ty),
        )
    }

    pub(super) fn place_length(
        &mut self,
        ty: TyId,
        metadata: Option<NodeId>,
    ) -> Result<NodeId, BuildError> {
        if let TypeShape::Array { length, .. } = self.graph.oracle.shape(ty) {
            let length = *length;
            let usize_ty = self.usize_type();
            Ok(self.graph.add_constant(u128::from(length), usize_ty))
        } else {
            metadata.ok_or_else(|| BuildError::Unsupported {
                block: self.current_block,
                what: "unsized place has no metadata".into(),
            })
        }
    }

    pub(super) fn read_place(
        &mut self,
        place: Place,
        block: usize,
        control: &mut NodeId,
    ) -> Result<NodeId, BuildError> {
        self.semantic_read_place(place, block, control)
    }
    pub(super) fn write_place(
        &mut self,
        place: Place,
        value: NodeId,
        block: usize,
        control: &mut NodeId,
    ) -> Result<(), BuildError> {
        self.semantic_write_place(place, value, block, control)
    }
    pub(super) fn operand(
        &mut self,
        operand: &Operand,
        block: usize,
        control: &mut NodeId,
    ) -> Result<NodeId, BuildError> {
        self.semantic_operand(operand, block, control)
    }
    pub(super) fn operand_type(&self, operand: &Operand) -> TyId {
        match operand {
            Operand::Copy(place) | Operand::Move(place) => {
                self.place_type(*place).expect("valid place")
            }
            Operand::Constant(id) => self.body.constant(*id).ty,
        }
    }

    pub(super) fn set_discriminant(
        &mut self,
        place: Place,
        variant: u32,
        block: usize,
        control: &mut NodeId,
    ) -> Result<(), BuildError> {
        let address = self.address(place, block, control)?;
        let effect = self.read_variable(self.effect, block);
        let usize_ty = self.usize_type();
        let variant = self.graph.add_constant(u128::from(variant), usize_ty);
        let set = self.graph.add_node(
            Opcode::SetVariant,
            *control,
            &[effect, address.address, variant],
            0,
            ValueType::Tuple(vec![ValueType::Control, ValueType::Effect]),
        );
        *control = self.projection(set, 0, ValueType::Control);
        let effect = self.projection(set, 1, ValueType::Effect);
        self.write_variable(self.effect, block, effect);
        Ok(())
    }

    pub(super) fn copy_nonoverlapping(
        &mut self,
        source: NodeId,
        destination: NodeId,
        count: NodeId,
        _element_size: u64,
        block: usize,
        control: &mut NodeId,
    ) -> Result<(), BuildError> {
        let effect = self.read_variable(self.effect, block);
        let copy = self.graph.add_node(
            Opcode::CopyNonOverlapping,
            *control,
            &[effect, source, destination, count],
            0,
            ValueType::Tuple(vec![ValueType::Control, ValueType::Effect]),
        );
        *control = self.projection(copy, 0, ValueType::Control);
        let effect = self.projection(copy, 1, ValueType::Effect);
        self.write_variable(self.effect, block, effect);
        Ok(())
    }
}
