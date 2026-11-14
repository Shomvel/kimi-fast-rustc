//! Pointer coercions and representation casts at the MIR boundary.

use super::Converter;
use kfrc_body::{
    CastKind, Constant, ConstantKind, FieldIndex, LocalDecl, Mutability, Operand, Place,
    ProjectionElem, Rvalue, StatementKind, TyId,
};
use rustc_middle::mir;
use rustc_middle::ty::adjustment::PointerCoercion;
use rustc_middle::ty::{self, Instance, Ty, TypingEnv};

impl<'tcx> Converter<'tcx> {
    pub(super) fn cast(
        &mut self,
        kind: mir::CastKind,
        operand: &mir::Operand<'tcx>,
        target: Ty<'tcx>,
    ) -> Result<Rvalue, String> {
        let body = self.tcx.instance_mir(self.instance.def);
        let source = self.monomorphize(operand.ty(&body.local_decls, self.tcx));
        let target = self.monomorphize(target);
        let source_ty = self.ty(source);
        let target_ty = self.ty(target);
        let reified = match kind {
            mir::CastKind::PointerCoercion(PointerCoercion::ReifyFnPointer(_), _) => {
                let ty::FnDef(def_id, args) = *source.kind() else {
                    return Err(format!(
                        "function-pointer reification requires a function item, not `{source}`"
                    ));
                };
                Some(
                    Instance::resolve_for_fn_ptr(
                        self.tcx,
                        TypingEnv::fully_monomorphized(),
                        def_id,
                        args,
                    )
                    .ok_or_else(|| format!("cannot resolve `{source}` as a function pointer"))?,
                )
            }
            mir::CastKind::PointerCoercion(PointerCoercion::ClosureFnPointer(_), _) => {
                let ty::Closure(def_id, args) = *source.kind() else {
                    return Err(format!(
                        "closure-pointer reification requires a closure, not `{source}`"
                    ));
                };
                Some(Instance::resolve_closure(
                    self.tcx,
                    def_id,
                    args,
                    ty::ClosureKind::FnOnce,
                ))
            }
            _ => None,
        };
        if let Some(instance) = reified {
            let symbol = self.tcx.symbol_name(instance).name.to_string();
            return Ok(Rvalue::Use(Operand::Constant(self.builder.add_constant(
                Constant {
                    ty: target_ty,
                    kind: ConstantKind::Address { symbol, addend: 0 },
                },
            ))));
        }
        let mut converted = self.operand(operand)?;
        if matches!(
            kind,
            mir::CastKind::PointerCoercion(PointerCoercion::Unsize, _)
        ) {
            return self.unsize_pointer(converted, source, target);
        }
        let source_wide = self.is_wide_pointer(source)?;
        let target_wide = self.is_wide_pointer(target)?;
        let pointer_cast = matches!(
            kind,
            mir::CastKind::PtrToPtr
                | mir::CastKind::FnPtrToPtr
                | mir::CastKind::PointerExposeProvenance
                | mir::CastKind::Subtype
                | mir::CastKind::PointerCoercion(
                    PointerCoercion::MutToConstPointer
                        | PointerCoercion::ArrayToPointer
                        | PointerCoercion::UnsafeFnPointer,
                    _
                )
        );
        if pointer_cast && source_wide {
            if target_wide {
                return Ok(Rvalue::Cast(CastKind::Transmute, converted, target_ty));
            }
            // Casting a wide pointer to a thin pointer discards metadata.
            let field_ty = self
                .builder
                .types()
                .aggregates
                .get(&source_ty)
                .and_then(|layout| layout.fields.first())
                .map(|field| field.ty)
                .ok_or_else(|| format!("no data field in pointer-pair `{source}`"))?;
            let place = match converted {
                Operand::Copy(place) | Operand::Move(place) => place,
                Operand::Constant(_) => {
                    let value = self.cast_temporary(source_ty, Rvalue::Use(converted));
                    let Operand::Copy(place) = value else {
                        unreachable!()
                    };
                    place
                }
            };
            let mut fields = self.builder.projection(place.projection).to_vec();
            fields.push(ProjectionElem::Field(FieldIndex(0), field_ty));
            converted = Operand::Copy(Place {
                local: place.local,
                projection: self.builder.add_projection(fields),
            });
        }
        let kind = match kind {
            mir::CastKind::IntToInt
            | mir::CastKind::PointerExposeProvenance
            | mir::CastKind::PointerWithExposedProvenance => CastKind::IntToInt,
            mir::CastKind::IntToFloat => CastKind::IntToFloat,
            mir::CastKind::FloatToInt => CastKind::FloatToInt,
            mir::CastKind::FloatToFloat => CastKind::FloatToFloat,
            mir::CastKind::Transmute | mir::CastKind::Subtype => CastKind::Transmute,
            _ if pointer_cast => CastKind::PtrToPtr,
            _ => CastKind::Other,
        };
        Ok(Rvalue::Cast(kind, converted, target_ty))
    }

    fn is_wide_pointer(&self, ty: Ty<'tcx>) -> Result<bool, String> {
        if pointer_pointee(ty).is_none() {
            return Ok(false);
        }
        let layout = self
            .tcx
            .layout_of(TypingEnv::fully_monomorphized().as_query_input(ty))
            .map_err(|error| format!("pointer layout for `{ty}`: {error:?}"))?;
        Ok(layout.size.bytes() == 16)
    }

    pub(super) fn cast_temporary(&mut self, ty: TyId, value: Rvalue) -> Operand {
        let local = self.builder.add_local(LocalDecl {
            ty,
            mutability: Mutability::Not,
            name: None,
        });
        self.builder
            .push_statement(StatementKind::Assign(Place::local(local), value));
        Operand::Copy(Place::local(local))
    }
}

pub(super) fn pointer_pointee(ty: Ty<'_>) -> Option<Ty<'_>> {
    match *ty.kind() {
        ty::Ref(_, pointee, _) | ty::RawPtr(pointee, _) => Some(pointee),
        _ => None,
    }
}
