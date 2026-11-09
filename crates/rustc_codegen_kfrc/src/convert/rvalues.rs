//! Translate MIR expressions without running optimization passes.

use super::Converter;
use kfrc_body::{
    AggregateKind, BinaryOp, BorrowKind, Constant, ConstantKind, Mutability, Operand, Place,
    Rvalue, UnaryOp, VariantIndex,
};
use rustc_middle::mir;
use rustc_middle::ty::Ty;

impl<'tcx> Converter<'tcx> {
    pub(super) fn rvalue(&mut self, rvalue: &mir::Rvalue<'tcx>) -> Result<Rvalue, String> {
        Ok(match rvalue {
            mir::Rvalue::Use(operand) => Rvalue::Use(self.operand(operand)?),
            mir::Rvalue::BinaryOp(op, operands) => {
                let (lhs, rhs) = &**operands;
                Rvalue::BinaryOp(binary_op(*op), self.operand(lhs)?, self.operand(rhs)?)
            }
            mir::Rvalue::UnaryOp(op, operand) => {
                let op = match op {
                    mir::UnOp::Not => UnaryOp::Not,
                    mir::UnOp::Neg => UnaryOp::Neg,
                    mir::UnOp::PtrMetadata => UnaryOp::PtrMetadata,
                };
                Rvalue::UnaryOp(op, self.operand(operand)?)
            }
            mir::Rvalue::Cast(kind, operand, ty) => self.cast(*kind, operand, *ty)?,
            mir::Rvalue::Ref(_, kind, place) => {
                let kind = match kind {
                    mir::BorrowKind::Shared => BorrowKind::Shared,
                    mir::BorrowKind::Fake(_) => BorrowKind::Fake,
                    mir::BorrowKind::Mut { .. } => BorrowKind::Mut,
                };
                Rvalue::Ref(kind, self.place(*place)?)
            }
            mir::Rvalue::RawPtr(kind, place) => {
                let mutability = if kind.to_mutbl_lossy() == mir::Mutability::Mut {
                    Mutability::Mut
                } else {
                    Mutability::Not
                };
                Rvalue::RawPointer(mutability, self.place(*place)?)
            }
            mir::Rvalue::Aggregate(kind, operands) => {
                let kind = match &**kind {
                    mir::AggregateKind::Tuple => AggregateKind::Tuple,
                    mir::AggregateKind::Array(ty) => {
                        AggregateKind::Array(self.ty(self.monomorphize(*ty)))
                    }
                    mir::AggregateKind::Adt(def_id, variant, args, _, active_field) => {
                        let ty = self
                            .monomorphize(self.tcx.type_of(*def_id).instantiate(self.tcx, args));
                        let ty_id = self.ty(ty);
                        if let Some(field) = active_field {
                            AggregateKind::Union {
                                ty: ty_id,
                                field: kfrc_body::FieldIndex(field.as_u32()),
                            }
                        } else {
                            AggregateKind::Adt {
                                ty: ty_id,
                                variant: VariantIndex(variant.as_u32()),
                            }
                        }
                    }
                    mir::AggregateKind::RawPtr(pointee, mutability) => {
                        let pointee = self.monomorphize(*pointee);
                        let mut converted = Vec::with_capacity(operands.len());
                        for operand in operands {
                            converted.push(self.operand(operand)?);
                        }
                        return if pointee
                            .is_sized(self.tcx, rustc_middle::ty::TypingEnv::fully_monomorphized())
                        {
                            let pointer = converted.first().copied().ok_or_else(|| {
                                "raw pointer aggregate has no data pointer".to_owned()
                            })?;
                            let ty = if *mutability == mir::Mutability::Mut {
                                Ty::new_mut_ptr(self.tcx, pointee)
                            } else {
                                Ty::new_imm_ptr(self.tcx, pointee)
                            };
                            Ok(Rvalue::Cast(
                                kfrc_body::CastKind::PtrToPtr,
                                pointer,
                                self.ty(ty),
                            ))
                        } else {
                            Ok(Rvalue::Aggregate(AggregateKind::Tuple, converted))
                        };
                    }
                    mir::AggregateKind::Closure(def_id, args) => AggregateKind::Closure(
                        self.ty(self.monomorphize(Ty::new_closure(self.tcx, *def_id, args))),
                    ),
                    _ => AggregateKind::Other,
                };
                let mut converted = Vec::with_capacity(operands.len());
                for operand in operands {
                    converted.push(self.operand(operand)?);
                }
                Rvalue::Aggregate(kind, converted)
            }
            mir::Rvalue::Discriminant(place) => {
                let body = self.tcx.instance_mir(self.instance.def);
                let ty = self.monomorphize(place.ty(&body.local_decls, self.tcx).ty);
                let place = self.place(*place)?;
                self.discriminant_rvalue(ty, place)
            }
            mir::Rvalue::Repeat(operand, length) => {
                let length = self
                    .monomorphize(*length)
                    .try_to_target_usize(self.tcx)
                    .ok_or_else(|| "array repeat length is not monomorphic".to_owned())?;
                let operand = self.operand(operand)?;
                Rvalue::Repeat(operand, length)
            }
            mir::Rvalue::ThreadLocalRef(def_id) => {
                if !self.tcx.is_thread_local_static(*def_id) {
                    return Err(format!(
                        "ThreadLocalRef does not name a thread-local static: `{}`",
                        self.tcx.def_path_str(*def_id)
                    ));
                }
                // The supported x86-64 Linux target exports native ELF TLS
                // directly, including across crates. Targets requiring a
                // ThreadLocalShim call need a different MIR call boundary.
                if !def_id.is_local() && self.tcx.needs_thread_local_shim(*def_id) {
                    return Err("thread-local shim requires an unsupported target".to_owned());
                }
                Rvalue::ThreadLocalRef {
                    symbol: self
                        .tcx
                        .symbol_name(rustc_middle::ty::Instance::mono(self.tcx, *def_id))
                        .name
                        .to_owned(),
                }
            }
            mir::Rvalue::ShallowInitBox(..) => Rvalue::Unsupported("ShallowInitBox".to_owned()),
            mir::Rvalue::CopyForDeref(place) => Rvalue::Use(Operand::Copy(self.place(*place)?)),
            mir::Rvalue::WrapUnsafeBinder(..) => Rvalue::Unsupported("unsafe binder".to_owned()),
        })
    }

    pub(super) fn discriminant_rvalue(&mut self, ty: Ty<'tcx>, place: Place) -> Rvalue {
        let mut base = ty;
        while let rustc_middle::ty::Pat(inner, _) = base.kind() {
            base = *inner;
        }
        if matches!(base.kind(), rustc_middle::ty::Adt(def, _) if def.is_enum())
            || matches!(base.kind(), rustc_middle::ty::Coroutine(..))
        {
            Rvalue::Discriminant(place)
        } else {
            // discriminant_value is defined for every T. rustc lowers it to
            // this MIR rvalue even when T is not an enum; that case is zero,
            // with T's associated DiscriminantKind type (normally u8).
            let ty = self.ty(ty.discriminant_ty(self.tcx));
            let zero = self.builder.add_constant(Constant {
                ty,
                kind: ConstantKind::Scalar(0),
            });
            Rvalue::Use(Operand::Constant(zero))
        }
    }
}

fn binary_op(op: mir::BinOp) -> BinaryOp {
    use mir::BinOp as B;
    match op {
        B::Add => BinaryOp::Add,
        B::AddUnchecked => BinaryOp::AddUnchecked,
        B::AddWithOverflow => BinaryOp::AddWithOverflow,
        B::Sub => BinaryOp::Sub,
        B::SubUnchecked => BinaryOp::SubUnchecked,
        B::SubWithOverflow => BinaryOp::SubWithOverflow,
        B::Mul => BinaryOp::Mul,
        B::MulUnchecked => BinaryOp::MulUnchecked,
        B::MulWithOverflow => BinaryOp::MulWithOverflow,
        B::Div => BinaryOp::Div,
        B::Rem => BinaryOp::Rem,
        B::BitXor => BinaryOp::BitXor,
        B::BitAnd => BinaryOp::BitAnd,
        B::BitOr => BinaryOp::BitOr,
        B::Shl => BinaryOp::Shl,
        B::ShlUnchecked => BinaryOp::ShlUnchecked,
        B::Shr => BinaryOp::Shr,
        B::ShrUnchecked => BinaryOp::ShrUnchecked,
        B::Eq => BinaryOp::Eq,
        B::Lt => BinaryOp::Lt,
        B::Le => BinaryOp::Le,
        B::Ne => BinaryOp::Ne,
        B::Ge => BinaryOp::Ge,
        B::Gt => BinaryOp::Gt,
        B::Cmp => BinaryOp::Cmp,
        B::Offset => BinaryOp::Offset,
    }
}
