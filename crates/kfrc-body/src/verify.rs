//! Structural checks on a Body before SoN construction.

use crate::body::{Body, BlockId, ConstantKind, Operand, Place, Rvalue, StatementKind, Terminator, UnwindAction};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BodyError {
    NoBlocks,
    BadArgCount { arg_count: usize, locals: usize },
    BadLocal { block: BlockId, local: u32 },
    BadBlock { block: BlockId, target: u32 },
    BadConstant { block: BlockId, constant: u32 },
    BadProjection { block: BlockId, projection: u32 },
    AbiArity { abi_args: usize, arg_count: usize },
    NonFunctionCallee { block: BlockId },
}

impl std::fmt::Display for BodyError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "invalid body: {self:?}")
    }
}

impl std::error::Error for BodyError {}

pub fn verify_body(body: &Body) -> Result<(), BodyError> {
    if body.blocks.is_empty() {
        return Err(BodyError::NoBlocks);
    }
    if body.arg_count + 1 > body.locals.len() {
        return Err(BodyError::BadArgCount {
            arg_count: body.arg_count,
            locals: body.locals.len(),
        });
    }
    if body.abi.args.len() != body.arg_count {
        return Err(BodyError::AbiArity {
            abi_args: body.abi.args.len(),
            arg_count: body.arg_count,
        });
    }
    for (index, block) in body.blocks.iter().enumerate() {
        let id = BlockId(index as u32);
        let check = Checker { body, block: id };
        for statement in body.block_statements(id) {
            match statement {
                StatementKind::Assign(place, rvalue) => {
                    check.place(*place)?;
                    check.rvalue(rvalue)?;
                }
                StatementKind::StorageLive(local) | StatementKind::StorageDead(local) => check.local(local.0)?,
                StatementKind::SetDiscriminant(place, _) => check.place(*place)?,
                StatementKind::Nop => {}
            }
        }
        for successor in block.terminator.successors() {
            check.block(successor)?;
        }
        if let Some(UnwindAction::Cleanup(cleanup)) = block.terminator.unwind() {
            check.block(cleanup)?;
        }
        match &block.terminator {
            Terminator::SwitchInt { discriminant, .. } => check.operand(discriminant)?,
            Terminator::Call { callee, args, destination, .. } => {
                check.operand(callee)?;
                for arg in args {
                    check.operand(arg)?;
                }
                check.place(*destination)?;
                if let Operand::Constant(constant) = callee
                    && !matches!(body.constant(*constant).kind, ConstantKind::Function(_))
                {
                    return Err(BodyError::NonFunctionCallee { block: id });
                }
            }
            Terminator::Assert { condition, .. } => check.operand(condition)?,
            Terminator::Drop { place, .. } => check.place(*place)?,
            _ => {}
        }
    }
    Ok(())
}

struct Checker<'a> {
    body: &'a Body,
    block: BlockId,
}

impl Checker<'_> {
    fn local(&self, local: u32) -> Result<(), BodyError> {
        if (local as usize) < self.body.locals.len() {
            Ok(())
        } else {
            Err(BodyError::BadLocal { block: self.block, local })
        }
    }

    fn block(&self, target: BlockId) -> Result<(), BodyError> {
        if target.index() < self.body.blocks.len() {
            Ok(())
        } else {
            Err(BodyError::BadBlock { block: self.block, target: target.0 })
        }
    }

    fn place(&self, place: Place) -> Result<(), BodyError> {
        self.local(place.local.0)?;
        if place.projection.index() >= self.body.projections.len() {
            return Err(BodyError::BadProjection { block: self.block, projection: place.projection.0 });
        }
        Ok(())
    }

    fn operand(&self, operand: &Operand) -> Result<(), BodyError> {
        match operand {
            Operand::Copy(place) | Operand::Move(place) => self.place(*place),
            Operand::Constant(constant) => {
                if constant.index() < self.body.constants.len() {
                    Ok(())
                } else {
                    Err(BodyError::BadConstant { block: self.block, constant: constant.0 })
                }
            }
        }
    }

    fn rvalue(&self, rvalue: &Rvalue) -> Result<(), BodyError> {
        match rvalue {
            Rvalue::Use(operand) | Rvalue::UnaryOp(_, operand) | Rvalue::Cast(_, operand, _) => self.operand(operand),
            Rvalue::BinaryOp(_, lhs, rhs) => {
                self.operand(lhs)?;
                self.operand(rhs)
            }
            Rvalue::Ref(_, place) | Rvalue::RawPointer(_, place) | Rvalue::Discriminant(place) | Rvalue::Len(place) => {
                self.place(*place)
            }
            Rvalue::Aggregate(_, operands) => operands.iter().try_for_each(|operand| self.operand(operand)),
            Rvalue::Unsupported(_) => Ok(()),
        }
    }
}
