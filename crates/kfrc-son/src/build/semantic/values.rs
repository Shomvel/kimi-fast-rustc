//! Construction of semantic aggregate, variant, and wide-pointer values.

use super::*;

impl Builder<'_> {
    pub(in super::super) fn semantic_rvalue(
        &mut self,
        rvalue: &Rvalue,
        ty: TyId,
        block: usize,
        control: &mut NodeId,
    ) -> Result<NodeId, BuildError> {
        if let Rvalue::Cast(CastKind::PtrToPtr, operand, target) = rvalue {
            let pointer = |ty| {
                matches!(
                    self.graph.oracle.shape(ty),
                    TypeShape::RawPointer { .. } | TypeShape::Reference { .. }
                )
            };
            if !pointer(self.operand_type(operand)) || !pointer(*target) {
                return self.unsupported("PtrToPtr requires pointer operands; exposed address conversions must use IntToInt");
            }
        }
        match rvalue {
            Rvalue::Aggregate(kind, operands) => {
                if let AggregateKind::Union {
                    ty: union_ty,
                    field,
                } = kind
                {
                    if *union_ty != ty
                        || self
                            .graph
                            .oracle
                            .semantic()
                            .maybe_uninit_inner(ty)
                            .is_none()
                    {
                        return self.unsupported("union construction is outside the semantic path");
                    }
                    if operands.len() != 1 {
                        return self
                            .unsupported("MaybeUninit constructor requires one selected field");
                    }
                    let field_ty = self
                        .graph
                        .oracle
                        .semantic()
                        .field_type(ty, None, field.0)
                        .ok_or_else(|| BuildError::Unsupported {
                            block: self.current_block,
                            what: "MaybeUninit constructor has invalid field".into(),
                        })?;
                    if field_ty != self.operand_type(&operands[0]) {
                        return self.unsupported("MaybeUninit constructor field type mismatch");
                    }
                    let value = self.operand(&operands[0], block, control)?;
                    return self.maybe_uninit_pack(value, ty, field_ty, block, control);
                }
                let variant = match kind {
                    AggregateKind::Adt { variant, .. }
                        if self.graph.oracle.semantic().is_enum(ty) =>
                    {
                        Some(variant.0)
                    }
                    _ => None,
                };
                if matches!(self.graph.oracle.shape(ty), TypeShape::Unit) && operands.is_empty() {
                    return Ok(self.graph.add_constant(0, ty));
                }
                if let TypeShape::Array { element, length } = self.graph.oracle.shape(ty) {
                    let (element, length) = (*element, *length);
                    if length != operands.len() as u64 {
                        return self
                            .unsupported("aggregate operand count disagrees with array length");
                    }
                    if length == 0 {
                        return self.semantic_empty_value(ty);
                    }
                    // Explicit array literals have one operand per element,
                    // but their common type need not be expanded into a second
                    // length-sized vector just to check those operands.
                    let mut values = Vec::with_capacity(operands.len());
                    for operand in operands {
                        if self.operand_type(operand) != element {
                            return self.unsupported("array operand has the wrong element type");
                        }
                        values.push(self.operand(operand, block, control)?);
                    }
                    return Ok(self.graph.add_node(
                        Opcode::Aggregate,
                        NodeId::NONE,
                        &values,
                        0,
                        ValueType::Rust(ty),
                    ));
                }
                let fields = self.semantic_fields(ty, variant)?;
                if fields.len() != operands.len() {
                    return self
                        .unsupported("aggregate operand count disagrees with semantic fields");
                }
                let mut values = Vec::with_capacity(operands.len());
                let wide = matches!(
                    self.graph.oracle.shape(ty),
                    TypeShape::Reference { .. } | TypeShape::RawPointer { .. }
                );
                for (index, (operand, field_ty)) in operands.iter().zip(fields).enumerate() {
                    let operand_ty = self.operand_type(operand);
                    let mut value = self.operand(operand, block, control)?;
                    if operand_ty != field_ty
                        && wide
                        && index == 0
                        && matches!(
                            self.graph.oracle.shape(operand_ty),
                            TypeShape::RawPointer { .. } | TypeShape::Reference { .. }
                        )
                        && self.semantic_scalar(operand_ty)
                    {
                        // The producer's wide data component may use a
                        // canonical erased pointee. This is a pointer retype,
                        // never an aggregate address or a layout operation.
                        value = self.pointer_cast(value, field_ty);
                    } else if operand_ty != field_ty {
                        return self
                            .unsupported("aggregate operand has the wrong semantic field type");
                    }
                    values.push(value);
                }
                let opcode = if variant.is_some() {
                    Opcode::MakeVariant
                } else if matches!(
                    self.graph.oracle.shape(ty),
                    TypeShape::Reference { .. } | TypeShape::RawPointer { .. }
                ) {
                    Opcode::WidePtr
                } else {
                    Opcode::Aggregate
                };
                Ok(self.graph.add_node(
                    opcode,
                    NodeId::NONE,
                    &values,
                    variant.unwrap_or(0),
                    ValueType::Rust(ty),
                ))
            }
            Rvalue::Repeat(operand, length) => {
                let TypeShape::Array {
                    element,
                    length: actual,
                } = self.graph.oracle.shape(ty)
                else {
                    return self.unsupported("Repeat requires a semantic array");
                };
                if *actual != *length || *element != self.operand_type(operand) {
                    return self.unsupported("Repeat disagrees with semantic array type");
                }
                let value = self.operand(operand, block, control)?;
                Ok(self.graph.add_node(
                    Opcode::Repeat,
                    NodeId::NONE,
                    &[value],
                    0,
                    ValueType::Rust(ty),
                ))
            }
            Rvalue::Discriminant(place) => {
                let value = self.read_place(*place, block, control)?;
                Ok(self.graph.add_node(
                    Opcode::Discriminant,
                    NodeId::NONE,
                    &[value],
                    0,
                    ValueType::Rust(ty),
                ))
            }
            Rvalue::Len(place) => {
                let ty = self.place_type(*place)?;
                if matches!(self.graph.oracle.shape(ty), TypeShape::Array { .. }) {
                    self.place_length(ty, None)
                } else {
                    let address = self.address(*place, block, control)?;
                    self.place_length(address.ty, address.metadata)
                }
            }
            Rvalue::UnaryOp(UnaryOp::PtrMetadata, operand) => {
                let source_ty = self.operand_type(operand);
                if !matches!(
                    self.graph.oracle.shape(source_ty),
                    TypeShape::Reference { .. } | TypeShape::RawPointer { .. }
                ) {
                    return self.unsupported("pointer metadata requires a semantic pointer value");
                }
                if self.semantic_scalar(source_ty) {
                    if matches!(self.graph.oracle.shape(ty), TypeShape::Unit) {
                        return Ok(self.graph.add_constant(0, ty));
                    }
                    return self.unsupported("thin pointer metadata must have unit type");
                }
                let value = self.operand(operand, block, control)?;
                self.semantic_field(value, source_ty, None, 1)
            }
            Rvalue::Ref(_, place) | Rvalue::RawPointer(_, place) => {
                let address = self.address(*place, block, control)?;
                if let Some(fields) = self.graph.oracle.semantic().field_types(ty, None) {
                    if fields.len() != 2 {
                        return self
                            .unsupported("wide reference must have two semantic components");
                    }
                    let data = self.pointer_cast(address.address, fields[0]);
                    let metadata = if let Some(metadata) = address.metadata {
                        metadata
                    } else {
                        self.place_length(address.ty, None)?
                    };
                    Ok(self.graph.add_node(
                        Opcode::WidePtr,
                        NodeId::NONE,
                        &[data, metadata],
                        0,
                        ValueType::Rust(ty),
                    ))
                } else {
                    Ok(self.pointer_cast(address.address, ty))
                }
            }
            Rvalue::ThreadLocalRef { .. } => self.unsupported("TLS is outside the semantic path"),
            Rvalue::BinaryOp(BinaryOp::Offset, lhs, rhs) => {
                let pointer = self.operand(lhs, block, control)?;
                let index = self.operand(rhs, block, control)?;
                let pointee = match self.graph.oracle.shape(ty) {
                    TypeShape::Reference { pointee, .. }
                    | TypeShape::RawPointer { pointee, .. } => *pointee,
                    _ => return self.unsupported("offset result must be a pointer"),
                };
                let address = self.index_address(pointer, index, pointee);
                Ok(self.pointer_cast(address, ty))
            }
            Rvalue::BinaryOp(
                BinaryOp::AddWithOverflow | BinaryOp::SubWithOverflow | BinaryOp::MulWithOverflow,
                ..,
            ) => self.semantic_checked_arithmetic(rvalue, ty, block, control),
            Rvalue::BinaryOp(op, left, right)
                if *op == BinaryOp::Cmp || !self.semantic_scalar(self.operand_type(left)) =>
            {
                self.semantic_compare(*op, left, right, ty, block, control)
            }
            Rvalue::Cast(CastKind::Transmute, operand, target) => {
                let value = self.operand(operand, block, control)?;
                Ok(self.semantic_transmute(value, *target, block, control))
            }
            Rvalue::Cast(CastKind::PtrToPtr, operand, target)
                if !self.semantic_scalar(self.operand_type(operand)) =>
            {
                let source_ty = self.operand_type(operand);
                let value = self.operand(operand, block, control)?;
                let data = self.semantic_field(value, source_ty, None, 0)?;
                if self.semantic_scalar(*target) {
                    Ok(self.pointer_cast(data, *target))
                } else {
                    let fields = self.semantic_fields(*target, None)?;
                    if fields.len() != 2 {
                        return self.unsupported("wide pointer cast requires two components");
                    }
                    let data = self.pointer_cast(data, fields[0]);
                    let metadata = self.semantic_field(value, source_ty, None, 1)?;
                    if self.graph.rust_type(metadata) != Some(fields[1]) {
                        return self.unsupported("wide pointer cast changes metadata type");
                    }
                    Ok(self.graph.add_node(
                        Opcode::WidePtr,
                        NodeId::NONE,
                        &[data, metadata],
                        0,
                        ValueType::Rust(*target),
                    ))
                }
            }
            Rvalue::Cast(_, operand, _)
                if !self.semantic_scalar(self.operand_type(operand))
                    || !self.semantic_scalar(ty) =>
            {
                self.unsupported("aggregate cast requires semantic representation conversion")
            }
            _ => self.rvalue(rvalue, ty, block, control),
        }
    }

    fn semantic_checked_arithmetic(
        &mut self,
        rvalue: &Rvalue,
        ty: TyId,
        block: usize,
        control: &mut NodeId,
    ) -> Result<NodeId, BuildError> {
        let Rvalue::BinaryOp(op, lhs, rhs) = rvalue else {
            unreachable!()
        };
        let input_ty = self.operand_type(lhs);
        let bool_ty = self.bool_type();
        if self.semantic_fields(ty, None)? != [input_ty, bool_ty] {
            return self.unsupported("checked arithmetic result must be (operand, bool)");
        }
        let left = self.operand(lhs, block, control)?;
        let right = self.operand(rhs, block, control)?;
        let opcode = match op {
            BinaryOp::AddWithOverflow => Opcode::Add,
            BinaryOp::SubWithOverflow => Opcode::Sub,
            _ => Opcode::Mul,
        };
        let result = self.graph.add_node(
            opcode,
            NodeId::NONE,
            &[left, right],
            0,
            ValueType::Rust(input_ty),
        );
        let overflow = if *op == BinaryOp::MulWithOverflow {
            self.graph.add_node(
                Opcode::MulOverflow,
                NodeId::NONE,
                &[left, right],
                0,
                ValueType::Rust(bool_ty),
            )
        } else if self.is_signed(input_ty) {
            let first = self.graph.add_node(
                Opcode::BitXor,
                NodeId::NONE,
                &[
                    left,
                    if *op == BinaryOp::AddWithOverflow {
                        result
                    } else {
                        right
                    },
                ],
                0,
                ValueType::Rust(input_ty),
            );
            let second = self.graph.add_node(
                Opcode::BitXor,
                NodeId::NONE,
                &[
                    if *op == BinaryOp::AddWithOverflow {
                        right
                    } else {
                        left
                    },
                    result,
                ],
                0,
                ValueType::Rust(input_ty),
            );
            let both = self.graph.add_node(
                Opcode::BitAnd,
                NodeId::NONE,
                &[first, second],
                0,
                ValueType::Rust(input_ty),
            );
            let zero = self.graph.add_constant(0, input_ty);
            self.graph.add_node(
                Opcode::Compare,
                NodeId::NONE,
                &[both, zero],
                ComparePredicate::SignedLess as u32,
                ValueType::Rust(bool_ty),
            )
        } else {
            let values = if *op == BinaryOp::AddWithOverflow {
                [result, left]
            } else {
                [left, right]
            };
            self.graph.add_node(
                Opcode::Compare,
                NodeId::NONE,
                &values,
                ComparePredicate::UnsignedLess as u32,
                ValueType::Rust(bool_ty),
            )
        };
        Ok(self.graph.add_node(
            Opcode::Aggregate,
            NodeId::NONE,
            &[result, overflow],
            0,
            ValueType::Rust(ty),
        ))
    }
}
