//! Rust comparison semantics, before enum tags and wide-pointer lanes are laid out.

use super::*;

impl Builder<'_> {
    pub(super) fn semantic_compare(
        &mut self,
        op: BinaryOp,
        lhs: &Operand,
        rhs: &Operand,
        ty: TyId,
        block: usize,
        control: &mut NodeId,
    ) -> Result<NodeId, BuildError> {
        let source_ty = self.operand_type(lhs);
        let left = self.operand(lhs, block, control)?;
        let right = self.operand(rhs, block, control)?;
        if op == BinaryOp::Cmp {
            if !self.semantic_scalar(source_ty) {
                return self.unsupported("Cmp operands must be scalar Rust values");
            }
            let types = self.graph.oracle.semantic();
            if !types.is_ordering(ty)
                || types.variant_count(ty) != Some(3)
                || (0..3).any(|variant| {
                    types
                        .field_types(ty, Some(variant))
                        .is_none_or(|fields| !fields.is_empty())
                })
            {
                return self.unsupported("Cmp requires the semantic core::cmp::Ordering enum");
            }
            let less_discriminant = types.discriminant(ty, 0);
            if types.discriminant(ty, 1) != Some(0)
                || types.discriminant(ty, 2) != Some(1)
                || !less_discriminant.is_some_and(|value| {
                    value == u128::MAX || (value >= 255 && value.wrapping_add(1).is_power_of_two())
                })
            {
                return self.unsupported("Ordering requires source discriminants -1, 0, 1");
            }
            // Ordering's declared variant identities are part of its language
            // contract, independent of its signed physical tag representation.
            let less = self.compare_scalar(left, right, BinaryOp::Lt, self.is_signed(source_ty));
            let greater = self.compare_scalar(left, right, BinaryOp::Gt, self.is_signed(source_ty));
            let less_value = self.graph.add_node(
                Opcode::MakeVariant,
                NodeId::NONE,
                &[],
                0,
                ValueType::Rust(ty),
            );
            let equal_value = self.graph.add_node(
                Opcode::MakeVariant,
                NodeId::NONE,
                &[],
                1,
                ValueType::Rust(ty),
            );
            let greater_value = self.graph.add_node(
                Opcode::MakeVariant,
                NodeId::NONE,
                &[],
                2,
                ValueType::Rust(ty),
            );
            let branch = self.graph.add_node(
                Opcode::If,
                *control,
                &[less],
                0,
                ValueType::Tuple(vec![ValueType::Control, ValueType::Control]),
            );
            let less_control = self.projection(branch, 0, ValueType::Control);
            let other = self.projection(branch, 1, ValueType::Control);
            let branch = self.graph.add_node(
                Opcode::If,
                other,
                &[greater],
                0,
                ValueType::Tuple(vec![ValueType::Control, ValueType::Control]),
            );
            let greater_control = self.projection(branch, 0, ValueType::Control);
            let equal_control = self.projection(branch, 1, ValueType::Control);
            let jumps = [less_control, equal_control, greater_control].map(|control| {
                self.graph
                    .add_node(Opcode::Jump, control, &[], 0, ValueType::Control)
            });
            let region =
                self.graph
                    .add_node(Opcode::Region, NodeId::NONE, &jumps, 0, ValueType::Control);
            *control = region;
            return Ok(self.graph.add_node(
                Opcode::Phi,
                NodeId::NONE,
                &[region, less_value, equal_value, greater_value],
                0,
                ValueType::Rust(ty),
            ));
        }
        if !matches!(
            self.graph.oracle.shape(source_ty),
            TypeShape::Reference { .. } | TypeShape::RawPointer { .. }
        ) {
            return self.unsupported("builtin comparison of a non-pointer aggregate");
        }
        if !matches!(
            op,
            BinaryOp::Eq | BinaryOp::Ne | BinaryOp::Lt | BinaryOp::Le | BinaryOp::Gt | BinaryOp::Ge
        ) {
            return self.unsupported("unsupported aggregate binary operator");
        }
        let mut lanes = Vec::with_capacity(2);
        for field in 0..2 {
            lanes.push((
                self.semantic_field(left, source_ty, None, field)?,
                self.semantic_field(right, source_ty, None, field)?,
            ));
        }
        let data_equal = self.compare_scalar(lanes[0].0, lanes[0].1, BinaryOp::Eq, false);
        let metadata_relation = self.compare_scalar(
            lanes[1].0,
            lanes[1].1,
            if op == BinaryOp::Ne { BinaryOp::Eq } else { op },
            false,
        );
        let equal_metadata = self.graph.add_node(
            Opcode::BitAnd,
            NodeId::NONE,
            &[data_equal, metadata_relation],
            0,
            ValueType::Rust(ty),
        );
        if matches!(op, BinaryOp::Eq | BinaryOp::Ne) {
            return if op == BinaryOp::Eq {
                Ok(equal_metadata)
            } else {
                Ok(self.graph.add_node(
                    Opcode::BitNot,
                    NodeId::NONE,
                    &[equal_metadata],
                    0,
                    ValueType::Rust(ty),
                ))
            };
        }
        let strict = if matches!(op, BinaryOp::Lt | BinaryOp::Le) {
            BinaryOp::Lt
        } else {
            BinaryOp::Gt
        };
        let data_relation = self.compare_scalar(lanes[0].0, lanes[0].1, strict, false);
        Ok(self.graph.add_node(
            Opcode::BitOr,
            NodeId::NONE,
            &[data_relation, equal_metadata],
            0,
            ValueType::Rust(ty),
        ))
    }

    fn compare_scalar(
        &mut self,
        left: NodeId,
        right: NodeId,
        op: BinaryOp,
        signed: bool,
    ) -> NodeId {
        let boolean = self.bool_type();
        self.graph.add_node(
            Opcode::Compare,
            NodeId::NONE,
            &[left, right],
            compare_predicate(op, signed) as u32,
            ValueType::Rust(boolean),
        )
    }
}
