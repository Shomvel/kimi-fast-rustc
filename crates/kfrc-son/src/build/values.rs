//! Scalar operations on Rust values, shared by all graph construction.

use super::*;

impl Builder<'_> {
    pub(super) fn is_signed(&self, ty: TyId) -> bool {
        matches!(
            self.graph.oracle.shape(ty),
            TypeShape::Int { signed: true, .. }
        )
    }

    pub(super) fn rvalue(
        &mut self,
        rvalue: &Rvalue,
        ty: TyId,
        block: usize,
        control: &mut NodeId,
    ) -> Result<NodeId, BuildError> {
        match rvalue {
            Rvalue::Use(operand) => self.operand(operand, block, control),
            Rvalue::Opaque(operand) => self.opaque(operand, ty, block, control),
            Rvalue::BinaryOp(op, lhs, rhs) => {
                let signed = self.is_signed(self.operand_type(lhs));
                let left = self.operand(lhs, block, control)?;
                let right = self.operand(rhs, block, control)?;
                let opcode = match op {
                    BinaryOp::Add | BinaryOp::AddUnchecked => Opcode::Add,
                    BinaryOp::Sub | BinaryOp::SubUnchecked => Opcode::Sub,
                    BinaryOp::Mul | BinaryOp::MulUnchecked => Opcode::Mul,
                    BinaryOp::Div if signed => Opcode::DivSigned,
                    BinaryOp::Div => Opcode::DivUnsigned,
                    BinaryOp::Rem if signed => Opcode::RemSigned,
                    BinaryOp::Rem => Opcode::RemUnsigned,
                    BinaryOp::BitAnd => Opcode::BitAnd,
                    BinaryOp::BitOr => Opcode::BitOr,
                    BinaryOp::BitXor => Opcode::BitXor,
                    BinaryOp::Shl | BinaryOp::ShlUnchecked => Opcode::ShiftLeft,
                    BinaryOp::Shr | BinaryOp::ShrUnchecked if signed => {
                        Opcode::ShiftRightArithmetic
                    }
                    BinaryOp::Shr | BinaryOp::ShrUnchecked => Opcode::ShiftRightLogical,
                    BinaryOp::Eq
                    | BinaryOp::Ne
                    | BinaryOp::Lt
                    | BinaryOp::Le
                    | BinaryOp::Gt
                    | BinaryOp::Ge => {
                        let predicate = compare_predicate(*op, signed);
                        return Ok(self.graph.add_node(
                            Opcode::Compare,
                            NodeId::NONE,
                            &[left, right],
                            predicate as u32,
                            ValueType::Rust(ty),
                        ));
                    }
                    BinaryOp::AddWithOverflow
                    | BinaryOp::SubWithOverflow
                    | BinaryOp::MulWithOverflow => {
                        return self.unsupported(format!("{op:?} (overflow checks produce a (value, bool) pair; build with -Coverflow-checks=off for P0)"));
                    }
                    BinaryOp::Cmp | BinaryOp::Offset => {
                        return self.unsupported(format!("binary operator {op:?}"));
                    }
                };
                // Division may trap: fix it on the control skeleton.
                let fixed_control = if opcode.is_fixed() {
                    *control
                } else {
                    NodeId::NONE
                };
                Ok(self.graph.add_node(
                    opcode,
                    fixed_control,
                    &[left, right],
                    0,
                    ValueType::Rust(ty),
                ))
            }
            Rvalue::UnaryOp(op, operand) => {
                let value = self.operand(operand, block, control)?;
                let opcode = match op {
                    UnaryOp::Not => Opcode::BitNot,
                    UnaryOp::Neg => Opcode::Negate,
                    UnaryOp::PtrMetadata => {
                        return self.unsupported("PtrMetadata must use the structured-value path");
                    }
                };
                Ok(self
                    .graph
                    .add_node(opcode, NodeId::NONE, &[value], 0, ValueType::Rust(ty)))
            }
            Rvalue::Cast(CastKind::IntToInt, operand, target) => {
                let value = self.operand(operand, block, control)?;
                Ok(self.graph.add_node(
                    Opcode::IntCast,
                    NodeId::NONE,
                    &[value],
                    0,
                    ValueType::Rust(*target),
                ))
            }
            Rvalue::Cast(CastKind::PtrToPtr, operand, target) => {
                let value = self.operand(operand, block, control)?;
                Ok(self.pointer_cast(value, *target))
            }
            Rvalue::Unsupported(what) => self.unsupported(what.clone()),
            _ => self.unsupported(format!("unhandled semantic rvalue {rvalue:?}")),
        }
    }
    pub(super) fn opaque(
        &mut self,
        operand: &Operand,
        ty: TyId,
        block: usize,
        control: &mut NodeId,
    ) -> Result<NodeId, BuildError> {
        let value = self.operand(operand, block, control)?;
        let effect = self.read_variable(self.effect, block);
        let barrier = self.graph.add_node(
            Opcode::Opaque,
            *control,
            &[effect, value],
            0,
            ValueType::Tuple(vec![
                ValueType::Control,
                ValueType::Effect,
                ValueType::Rust(ty),
            ]),
        );
        *control = self.projection(barrier, 0, ValueType::Control);
        let effect = self.projection(barrier, 1, ValueType::Effect);
        self.write_variable(self.effect, block, effect);
        Ok(self.projection(barrier, 2, ValueType::Rust(ty)))
    }
}
