//! Control-flow edges, drop glue, and explicit panic failure blocks.

use super::Converter;
use kfrc_body::{
    BlockId, Callee, Constant, ConstantKind, LocalDecl, Mutability, Operand, Place, Rvalue,
    StatementKind, SwitchTargets, Terminator, UnwindAction,
};
use rustc_middle::mir;
use rustc_middle::ty::{self, Instance, InstanceKind, Ty};

impl<'tcx> Converter<'tcx> {
    pub(super) fn unwind(&self, action: &mir::UnwindAction) -> UnwindAction {
        match action {
            mir::UnwindAction::Continue => UnwindAction::Continue,
            mir::UnwindAction::Unreachable => UnwindAction::Unreachable,
            mir::UnwindAction::Terminate(_) => UnwindAction::Terminate,
            mir::UnwindAction::Cleanup(block) => UnwindAction::Cleanup(BlockId(block.as_u32())),
        }
    }

    pub(super) fn terminator(
        &mut self,
        terminator: &mir::Terminator<'tcx>,
    ) -> Result<Terminator, String> {
        let block = |target: &mir::BasicBlock| BlockId(target.as_u32());
        Ok(match &terminator.kind {
            mir::TerminatorKind::Goto { target } => Terminator::Goto(block(target)),
            mir::TerminatorKind::FalseEdge { real_target, .. }
            | mir::TerminatorKind::FalseUnwind { real_target, .. } => {
                Terminator::Goto(block(real_target))
            }
            mir::TerminatorKind::SwitchInt { discr, targets } => Terminator::SwitchInt {
                discriminant: self.operand(discr)?,
                targets: SwitchTargets {
                    cases: targets
                        .iter()
                        .map(|(value, target)| (value, block(&target)))
                        .collect(),
                    otherwise: block(&targets.otherwise()),
                },
            },
            mir::TerminatorKind::Return => Terminator::Return,
            mir::TerminatorKind::Unreachable => Terminator::Unreachable,
            mir::TerminatorKind::UnwindResume => Terminator::UnwindResume,
            mir::TerminatorKind::UnwindTerminate(_) => Terminator::UnwindTerminate,
            mir::TerminatorKind::Drop {
                place,
                target,
                unwind,
                ..
            } => {
                let mir = self.tcx.instance_mir(self.instance.def);
                let dropped_ty = self.monomorphize(place.ty(&mir.local_decls, self.tcx).ty);
                if matches!(dropped_ty.kind(), ty::Dynamic(..)) {
                    return self.dynamic_drop(
                        *place,
                        dropped_ty,
                        block(target),
                        self.unwind(unwind),
                    );
                }
                let instance = Instance::resolve_drop_in_place(self.tcx, dropped_ty);
                if matches!(instance.def, InstanceKind::DropGlue(_, None)) {
                    return Ok(Terminator::Goto(block(target)));
                }
                let ptr_ty = self.ty(Ty::new_mut_ptr(self.tcx, dropped_ty));
                let pointer = self.builder.add_local(LocalDecl {
                    ty: ptr_ty,
                    mutability: Mutability::Not,
                    name: None,
                });
                let dropped_place = self.place(*place)?;
                self.builder.push_statement(StatementKind::Assign(
                    Place::local(pointer),
                    Rvalue::RawPointer(Mutability::Mut, dropped_place),
                ));
                let abi = crate::abi::instance_abi(self.tcx, instance)?;
                let mut args = vec![Operand::Copy(Place::local(pointer))];
                if instance.def.requires_caller_location(self.tcx) {
                    args.push(self.caller_location(terminator.source_info)?);
                }
                let fn_ty = self.ty(Ty::new_fn_def(self.tcx, instance.def_id(), instance.args));
                let callee = Operand::Constant(self.builder.add_constant(Constant {
                    ty: fn_ty,
                    kind: ConstantKind::Function(Callee::Function {
                        symbol: self.tcx.symbol_name(instance).name.to_string(),
                        abi,
                    }),
                }));
                let unit_ty = self.ty(self.tcx.types.unit);
                let result = self.builder.add_local(LocalDecl {
                    ty: unit_ty,
                    mutability: Mutability::Not,
                    name: None,
                });
                Terminator::Call {
                    callee,
                    args,
                    destination: Place::local(result),
                    target: Some(block(target)),
                    unwind: self.unwind(unwind),
                }
            }
            mir::TerminatorKind::Call { .. } => return self.call_terminator(terminator),
            mir::TerminatorKind::Assert {
                cond,
                expected,
                msg,
                target,
                ..
            } => {
                if !self.tcx.sess.overflow_checks() && msg.is_optional_overflow_check() {
                    return Ok(Terminator::Goto(block(target)));
                }
                let condition = self.operand(cond)?;
                let mut args = Vec::new();
                let lang_item = match &**msg {
                    mir::AssertKind::BoundsCheck { len, index } => {
                        args.push(self.operand(index)?);
                        args.push(self.operand(len)?);
                        rustc_hir::LangItem::PanicBoundsCheck
                    }
                    mir::AssertKind::MisalignedPointerDereference { required, found } => {
                        args.push(self.operand(required)?);
                        args.push(self.operand(found)?);
                        rustc_hir::LangItem::PanicMisalignedPointerDereference
                    }
                    mir::AssertKind::NullPointerDereference => {
                        rustc_hir::LangItem::PanicNullPointerDereference
                    }
                    mir::AssertKind::InvalidEnumConstruction(source) => {
                        args.push(self.operand(source)?);
                        rustc_hir::LangItem::PanicInvalidEnumConstruction
                    }
                    other => other.panic_function(),
                };
                args.push(self.caller_location(terminator.source_info)?);
                let def_id = self
                    .tcx
                    .require_lang_item(lang_item, terminator.source_info.span);
                let callee = self.callee(def_id, ty::List::empty(), terminator.source_info.span)?;
                let function_ty = self.ty(Ty::new_fn_def(
                    self.tcx,
                    def_id,
                    ty::List::<ty::GenericArg<'tcx>>::empty(),
                ));
                let callee = Operand::Constant(self.builder.add_constant(Constant {
                    ty: function_ty,
                    kind: ConstantKind::Function(callee),
                }));
                let failure = BlockId((self.original_blocks + self.pending_blocks.len()) as u32);
                let never_ty = self.ty(self.tcx.types.never);
                let result = self.builder.add_local(LocalDecl {
                    ty: never_ty,
                    mutability: Mutability::Not,
                    name: None,
                });
                self.pending_blocks.push((
                    Vec::new(),
                    Terminator::Call {
                        callee,
                        args,
                        destination: Place::local(result),
                        target: None,
                        unwind: UnwindAction::Unreachable,
                    },
                ));
                Terminator::SwitchInt {
                    discriminant: condition,
                    targets: SwitchTargets {
                        cases: vec![(u128::from(*expected), block(target))],
                        otherwise: failure,
                    },
                }
            }
            other => return Err(format!("terminator {:?}", other.name())),
        })
    }
}
