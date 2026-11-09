//! Typed temporaries and libc calls used by intrinsic lowering.

use super::*;

impl<'tcx> Converter<'tcx> {
    pub(super) fn intrinsic_result_ty(&self, instance: Instance<'tcx>) -> Ty<'tcx> {
        let signature = self
            .tcx
            .fn_sig(instance.def_id())
            .instantiate(self.tcx, instance.args)
            .skip_binder();
        self.monomorphize(signature.output())
    }

    pub(super) fn intrinsic_temp(&mut self, ty: Ty<'tcx>, value: Rvalue) -> Operand {
        let ty = self.ty(ty);
        let local = self.builder.add_local(LocalDecl {
            ty,
            mutability: Mutability::Mut,
            name: None,
        });
        self.builder
            .push_statement(StatementKind::Assign(Place::local(local), value));
        Operand::Copy(Place::local(local))
    }

    pub(super) fn intrinsic_constant(&mut self, ty: Ty<'tcx>, value: u128) -> Operand {
        let ty = self.ty(ty);
        Operand::Constant(self.builder.add_constant(Constant {
            ty,
            kind: ConstantKind::Scalar(value),
        }))
    }

    pub(super) fn intrinsic_cast(&mut self, value: Operand, ty: Ty<'tcx>) -> Operand {
        let target = self.ty(ty);
        self.intrinsic_temp(ty, Rvalue::Cast(CastKind::IntToInt, value, target))
    }

    pub(super) fn intrinsic_layout(&self, ty: Ty<'tcx>) -> Result<(u64, u64), String> {
        let layout = self
            .tcx
            .layout_of(TypingEnv::fully_monomorphized().as_query_input(ty))
            .map_err(|error| format!("intrinsic has no layout for `{ty}`: {error:?}"))?;
        if !layout.is_sized() {
            return Err(format!("intrinsic requires sized type `{ty}`"));
        }
        Ok((layout.size.bytes(), layout.align.abi.bytes()))
    }

    pub(super) fn intrinsic_scale(
        &mut self,
        count: Operand,
        element: Ty<'tcx>,
    ) -> Result<Operand, String> {
        let size = self.intrinsic_layout(element)?.0;
        let count = self.intrinsic_cast(count, self.tcx.types.usize);
        let size = self.intrinsic_constant(self.tcx.types.usize, u128::from(size));
        Ok(self.intrinsic_temp(
            self.tcx.types.usize,
            Rvalue::BinaryOp(BinaryOp::Mul, count, size),
        ))
    }

    pub(super) fn intrinsic_deref(&mut self, pointer: Operand, pointee: Ty<'tcx>) -> Place {
        let pointer_ty = Ty::new_mut_ptr(self.tcx, pointee);
        let pointer_id = self.ty(pointer_ty);
        let pointer = self.intrinsic_temp(
            pointer_ty,
            Rvalue::Cast(CastKind::PtrToPtr, pointer, pointer_id),
        );
        let Operand::Copy(place) = pointer else {
            unreachable!()
        };
        let projection = self.builder.add_projection(vec![ProjectionElem::Deref]);
        Place {
            local: place.local,
            projection,
        }
    }

    pub(super) fn intrinsic_libcall(
        &mut self,
        symbol: &str,
        args: Vec<Operand>,
        result: Option<Ty<'tcx>>,
        target: Option<BlockId>,
        unwind: UnwindAction,
    ) -> Terminator {
        let result = result.unwrap_or(self.tcx.types.unit);
        let result_id = self.ty(result);
        let destination = self.builder.add_local(LocalDecl {
            ty: result_id,
            mutability: Mutability::Mut,
            name: None,
        });
        let fn_ty = self.ty(self.tcx.types.unit);
        let callee = self.builder.add_constant(Constant {
            ty: fn_ty,
            kind: ConstantKind::Function(Callee::Function {
                symbol: symbol.to_owned(),
                abi: FunctionAbi {
                    args: vec![PassMode::Direct; args.len()],
                    ret: if result.is_unit() {
                        PassMode::Ignore
                    } else {
                        PassMode::Direct
                    },
                    c_variadic: false,
                },
            }),
        });
        Terminator::Call {
            callee: Operand::Constant(callee),
            args,
            destination: Place::local(destination),
            target,
            unwind,
        }
    }

    pub(super) fn intrinsic_unsigned(&self, ty: Ty<'tcx>) -> Result<(Ty<'tcx>, u32), String> {
        if !matches!(ty.kind(), ty::Int(_) | ty::Uint(_)) {
            return Err(format!("bit intrinsic requires integer type, got `{ty}`"));
        }
        // In particular, isize/usize follow the compilation target's data layout.
        let bits = self.intrinsic_layout(ty)?.0 * 8;
        let unsigned = match bits {
            8 => self.tcx.types.u8,
            16 => self.tcx.types.u16,
            32 => self.tcx.types.u32,
            64 => self.tcx.types.u64,
            _ => {
                return Err(format!(
                    "bit intrinsic on {bits}-bit integer is not implemented"
                ));
            }
        };
        Ok((unsigned, bits as u32))
    }
}
