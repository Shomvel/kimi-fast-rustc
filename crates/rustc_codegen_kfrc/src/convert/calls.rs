//! Resolve call targets and map MIR arguments to the rustc call ABI.

use super::Converter;
use kfrc_body::{
    Callee, Constant, ConstantKind, FieldIndex, Operand, Place, ProjectionElem, Rvalue, Terminator,
};
use rustc_middle::mir;
use rustc_middle::ty::{self, Instance, InstanceKind, TypingEnv};

impl<'tcx> Converter<'tcx> {
    pub(super) fn callee(
        &mut self,
        def_id: rustc_span::def_id::DefId,
        args: ty::GenericArgsRef<'tcx>,
        span: rustc_span::Span,
    ) -> Result<Callee, String> {
        let instance = Instance::expect_resolve(
            self.tcx,
            TypingEnv::fully_monomorphized(),
            def_id,
            args,
            span,
        );
        match instance.def {
            InstanceKind::Intrinsic(def_id) => Ok(Callee::Intrinsic {
                name: self.tcx.item_name(def_id).to_string(),
            }),
            InstanceKind::Virtual(..) => Err(format!(
                "virtual call `{instance}` (dyn arrives as an extension goal)"
            )),
            _ => self.function_callee(instance),
        }
    }

    fn function_callee(&self, instance: Instance<'tcx>) -> Result<Callee, String> {
        let symbol = self.tcx.symbol_name(instance).name.to_string();
        if symbol.starts_with("llvm.") {
            return Err(format!("LLVM intrinsic `{symbol}`"));
        }
        let abi = crate::abi::instance_abi(self.tcx, instance)?;
        Ok(Callee::Function { symbol, abi })
    }

    pub(super) fn call_terminator(
        &mut self,
        terminator: &mir::Terminator<'tcx>,
    ) -> Result<Terminator, String> {
        let mir::TerminatorKind::Call {
            func,
            args,
            destination,
            target,
            unwind,
            ..
        } = &terminator.kind
        else {
            return Err("call converter received a non-call terminator".into());
        };
        let block = |target: &mir::BasicBlock| kfrc_body::BlockId(target.as_u32());
        let mir = self.tcx.instance_mir(self.instance.def);
        let function_ty = self.monomorphize(func.ty(&mir.local_decls, self.tcx));
        let tuple_fields = crate::abi::call_tuple_fields(self.tcx, function_ty)?;
        let mut converted = Vec::with_capacity(args.len());
        for (index, arg) in args.iter().enumerate() {
            let operand = self.operand(&arg.node)?;
            if index + 1 == args.len()
                && let Some(fields) = &tuple_fields
            {
                if fields.is_empty() {
                    continue;
                }
                let place = match operand {
                    Operand::Copy(place) | Operand::Move(place) => place,
                    Operand::Constant(_) => {
                        // CTFE may represent a nonempty tuple as one scalar
                        // (e.g. (&mut Formatter,)) or as a zero-sized value.
                        // Give it typed storage before projecting its fields;
                        // indirect and pair constants already arrive in places.
                        let tuple_ty = self.monomorphize(arg.node.ty(&mir.local_decls, self.tcx));
                        let tuple_ty = self.ty(tuple_ty);
                        let Operand::Copy(place) =
                            self.cast_temporary(tuple_ty, Rvalue::Use(operand))
                        else {
                            unreachable!()
                        };
                        place
                    }
                };
                for (field, ty) in fields.iter().enumerate() {
                    let field_ty = self.ty(*ty);
                    let mut projection = self.builder.projection(place.projection).to_vec();
                    projection.push(ProjectionElem::Field(FieldIndex(field as u32), field_ty));
                    let projection = self.builder.add_projection(projection);
                    converted.push(Operand::Copy(Place {
                        local: place.local,
                        projection,
                    }));
                }
            } else {
                converted.push(operand);
            }
        }
        if let ty::FnDef(def_id, generic_args) = *function_ty.kind() {
            let instance = Instance::expect_resolve(
                self.tcx,
                TypingEnv::fully_monomorphized(),
                def_id,
                generic_args,
                terminator.source_info.span,
            );
            if matches!(instance.def, InstanceKind::Intrinsic(_)) {
                let destination = self.place(*destination)?;
                return self.intrinsic_call(
                    instance,
                    converted,
                    args,
                    destination,
                    target.as_ref().map(block),
                    self.unwind(unwind),
                    terminator.source_info,
                );
            }
            if matches!(instance.def, InstanceKind::DropGlue(_, None)) {
                // rustc's collector omits no-op drop glue for direct calls,
                // while still collecting it when its address is taken.
                return target
                    .as_ref()
                    .map(|target| Terminator::Goto(block(target)))
                    .ok_or_else(|| "trivial drop_in_place call has no successor".to_owned());
            }
            if matches!(instance.def, InstanceKind::Virtual(..)) {
                let receiver = args.first().ok_or("virtual call has no receiver")?;
                let receiver_ty = self.monomorphize(receiver.node.ty(&mir.local_decls, self.tcx));
                let destination = self.place(*destination)?;
                return self.virtual_call(
                    instance,
                    receiver_ty,
                    converted,
                    destination,
                    target.as_ref().map(block),
                    self.unwind(unwind),
                    terminator.source_info,
                );
            }
            // FnDef values are zero-sized even when held in a MIR local or
            // projected aggregate field. Their nominal type identifies the
            // direct target; reading their operand cannot supply an address.
            let callee = self.function_callee(instance)?;
            let Callee::Function { abi, symbol } = &callee else {
                unreachable!()
            };
            if instance.def.requires_caller_location(self.tcx) {
                converted.push(self.caller_location(terminator.source_info)?);
            }
            if abi.args.len() != converted.len() {
                return Err(format!(
                    "call to `{symbol}` passes {} arguments but its ABI has {}",
                    converted.len(),
                    abi.args.len()
                ));
            }
            let ty = self.ty(function_ty);
            let callee = Operand::Constant(self.builder.add_constant(Constant {
                ty,
                kind: ConstantKind::Function(callee),
            }));
            return Ok(Terminator::Call {
                callee,
                args: converted,
                destination: self.place(*destination)?,
                target: target.as_ref().map(block),
                unwind: self.unwind(unwind),
            });
        }
        // call_tuple_fields already rejects non-function types. A genuine
        // function pointer carries its dynamic address and a pointer ABI;
        // track-caller adapters are supplied when the pointer is reified.
        let function = self.operand(func)?;
        Ok(Terminator::CallIndirect {
            function,
            abi: crate::abi::fn_pointer_abi(self.tcx, function_ty.fn_sig(self.tcx))?,
            args: converted,
            destination: self.place(*destination)?,
            target: target.as_ref().map(block),
            unwind: self.unwind(unwind),
        })
    }
}
