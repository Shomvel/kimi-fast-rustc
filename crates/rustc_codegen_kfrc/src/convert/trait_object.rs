//! Trait-object dispatch through rustc's vtable allocation and slot indices.

use super::Converter;
use kfrc_body::{
    BinaryOp, BlockId, CastKind, Constant, ConstantKind, FieldIndex, LocalDecl, Mutability,
    Operand, PassMode, Place, ProjectionElem, Rvalue, StatementKind, SwitchTargets, Terminator,
    UnwindAction,
};
use rustc_middle::mir;
use rustc_middle::ty::{self, Instance, InstanceKind, Ty, TypingEnv};

impl<'tcx> Converter<'tcx> {
    /// Rustc normalizes the first virtual ABI argument to a thin data pointer.
    /// The MIR operand still has its original wide-pointer representation.
    pub(super) fn virtual_call(
        &mut self,
        instance: Instance<'tcx>,
        receiver_ty: Ty<'tcx>,
        mut args: Vec<Operand>,
        destination: Place,
        target: Option<BlockId>,
        unwind: UnwindAction,
        source_info: mir::SourceInfo,
    ) -> Result<Terminator, String> {
        let InstanceKind::Virtual(_, slot) = instance.def else {
            return Err("virtual dispatch received a nonvirtual instance".into());
        };
        let receiver = *args.first().ok_or("virtual call has no self argument")?;
        let (data, vtable) = self.object_parts(receiver, receiver_ty)?;
        let abi = crate::abi::instance_abi(self.tcx, instance)?;
        if abi.args.first() != Some(&PassMode::Direct) {
            return Err(format!(
                "virtual self ABI is not a thin pointer: {:?}",
                abi.args.first()
            ));
        }
        args[0] = data;
        if instance.def.requires_caller_location(self.tcx) {
            args.push(self.caller_location(source_info)?);
        }
        if args.len() != abi.args.len() {
            return Err(format!(
                "virtual `{instance}` has {} arguments but ABI requires {}",
                args.len(),
                abi.args.len()
            ));
        }
        let function = self.vtable_function(vtable, slot)?;
        Ok(Terminator::CallIndirect {
            function,
            abi,
            args,
            destination,
            target,
            unwind,
        })
    }

    /// Split a receiver after unwrapping transparent DispatchFromDyn wrappers
    /// (Pin, Box, Rc-style receivers) using rustc's actual non-ZST field layout.
    pub(super) fn object_parts(
        &mut self,
        mut operand: Operand,
        mut ty: Ty<'tcx>,
    ) -> Result<(Operand, Operand), String> {
        if matches!(ty.kind(), ty::Dynamic(..)) {
            let place = match operand {
                Operand::Copy(place) | Operand::Move(place) => place,
                Operand::Constant(_) => {
                    return Err("by-value dynamic receiver must be an unsized place".into());
                }
            };
            // A virtual by-value Self (notably dyn FnOnce) is an unsized
            // place, not a fat-pointer value to load. rustc's SSA backend uses
            // Ref(PlaceValue { llval: data, llextra: Some(vtable) }) here and
            // make_thin_self_ptr passes just that address in the virtual ABI.
            // Reborrow its existing address/metadata through RawPointer: the
            // Typed Body wide-pointer lowering performs no payload copy or
            // size-dependent load and preserves the originating projection.
            ty = Ty::new_mut_ptr(self.tcx, ty);
            operand = self.object_temp(ty, Rvalue::RawPointer(Mutability::Mut, place));
        }
        let cx =
            rustc_middle::ty::layout::LayoutCx::new(self.tcx, TypingEnv::fully_monomorphized());
        while !matches!(ty.kind(), ty::Ref(..) | ty::RawPtr(..)) {
            let layout = self
                .tcx
                .layout_of(TypingEnv::fully_monomorphized().as_query_input(ty))
                .map_err(|error| format!("virtual receiver layout `{ty}`: {error:?}"))?;
            let (field, field_layout) = layout.non_1zst_field(&cx).ok_or_else(|| {
                format!("virtual receiver `{ty}` is not a single-pointer wrapper")
            })?;
            operand = self.object_field(operand, ty, field.as_usize(), field_layout.ty)?;
            ty = field_layout.ty;
        }
        let pointee = ty
            .builtin_deref(true)
            .ok_or("virtual receiver is not a built-in pointer")?;
        if !matches!(pointee.kind(), ty::Dynamic(..)) {
            return Err(format!(
                "virtual receiver `{ty}` does not point to a trait object"
            ));
        }
        let layout = self
            .tcx
            .layout_of(TypingEnv::fully_monomorphized().as_query_input(ty))
            .map_err(|error| format!("virtual pointer layout `{ty}`: {error:?}"))?;
        // rustc's field(0) intentionally retains the *wide* pointer Ty while
        // overriding only its TyAndLayout representation to a thin pointer
        // (rustc_middle::ty::layout). The standalone Ty cannot represent that
        // override, so use the same canonical thin type as RecordedTypes.
        let data_ty = Ty::new_imm_ptr(self.tcx, self.tcx.types.unit);
        let vtable_ty = layout.field(&cx, 1).ty;
        Ok((
            self.object_field(operand, ty, 0, data_ty)?,
            self.object_field(operand, ty, 1, vtable_ty)?,
        ))
    }

    /// A vtable word is pointer-sized on the selected target. `slot` is the
    /// actual InstanceKind::Virtual index or rustc's common metadata constant.
    pub(super) fn vtable_slot(&mut self, vtable: Operand, slot: usize) -> Result<Operand, String> {
        self.typed_vtable_slot(vtable, slot, self.tcx.types.usize)
    }

    fn vtable_function(&mut self, vtable: Operand, slot: usize) -> Result<Operand, String> {
        let function_ty = Ty::new_imm_ptr(self.tcx, self.tcx.types.unit);
        self.typed_vtable_slot(vtable, slot, function_ty)
    }

    fn typed_vtable_slot(
        &mut self,
        vtable: Operand,
        slot: usize,
        value_ty: Ty<'tcx>,
    ) -> Result<Operand, String> {
        // Size/align words are integers; dispatch/drop entries are addresses.
        // Preserve that distinction and element indexing until layout lowering.
        let pointer_ty = Ty::new_imm_ptr(self.tcx, value_ty);
        let pointer_id = self.ty(pointer_ty);
        let base = self.object_temp(
            pointer_ty,
            Rvalue::Cast(CastKind::PtrToPtr, vtable, pointer_id),
        );
        let index_ty = self.ty(self.tcx.types.usize);
        let index = Operand::Constant(self.builder.add_constant(Constant {
            ty: index_ty,
            kind: ConstantKind::Scalar(slot as u128),
        }));
        let pointer = self.object_temp(pointer_ty, Rvalue::BinaryOp(BinaryOp::Offset, base, index));
        let Operand::Copy(pointer) = pointer else {
            unreachable!()
        };
        let projection = self.builder.add_projection(vec![ProjectionElem::Deref]);
        Ok(self.object_temp(
            value_ty,
            Rvalue::Use(Operand::Copy(Place {
                local: pointer.local,
                projection,
            })),
        ))
    }

    pub(super) fn dynamic_drop(
        &mut self,
        place: mir::Place<'tcx>,
        dropped_ty: Ty<'tcx>,
        target: BlockId,
        unwind: UnwindAction,
    ) -> Result<Terminator, String> {
        let Some((&mir::ProjectionElem::Deref, prefix)) = place.projection.split_last() else {
            return Err("dynamic drop requires a dereferenced trait-object pointer".into());
        };
        let pointer = mir::Place {
            local: place.local,
            projection: self.tcx.mk_place_elems(prefix),
        };
        let mir = self.tcx.instance_mir(self.instance.def);
        let pointer_ty = self.monomorphize(pointer.ty(&mir.local_decls, self.tcx).ty);
        let pointer = Operand::Copy(self.place(pointer)?);
        let (data, vtable) = self.object_parts(pointer, pointer_ty)?;
        let function = self.vtable_function(vtable, ty::COMMON_VTABLE_ENTRIES_DROPINPLACE)?;
        let drop = Instance::resolve_drop_in_place(self.tcx, dropped_ty);
        let virtual_drop = Instance {
            def: InstanceKind::Virtual(drop.def_id(), ty::COMMON_VTABLE_ENTRIES_DROPINPLACE),
            args: drop.args,
        };
        let abi = crate::abi::instance_abi(self.tcx, virtual_drop)?;
        if abi.args != [PassMode::Direct] || abi.ret != PassMode::Ignore {
            return Err(format!("unexpected dynamic drop ABI {abi:?}"));
        }
        let unit_ty = self.ty(self.tcx.types.unit);
        let result = self.builder.add_local(LocalDecl {
            ty: unit_ty,
            mutability: Mutability::Mut,
            name: None,
        });
        let invoke = BlockId((self.original_blocks + self.pending_blocks.len()) as u32);
        self.pending_blocks.push((
            Vec::new(),
            Terminator::CallIndirect {
                function,
                abi,
                args: vec![data],
                destination: Place::local(result),
                target: Some(target),
                unwind,
            },
        ));
        // Trivial drop slots are null in rustc's real vtables.
        Ok(Terminator::SwitchInt {
            discriminant: function,
            targets: SwitchTargets {
                cases: vec![(0, target)],
                otherwise: invoke,
            },
        })
    }

    fn object_field(
        &mut self,
        operand: Operand,
        ty: Ty<'tcx>,
        index: usize,
        field_ty: Ty<'tcx>,
    ) -> Result<Operand, String> {
        let operand = match operand {
            Operand::Copy(place) | Operand::Move(place) => place,
            Operand::Constant(_) => {
                let Operand::Copy(place) = self.object_temp(ty, Rvalue::Use(operand)) else {
                    unreachable!()
                };
                place
            }
        };
        let field_ty = self.ty(field_ty);
        let mut projection = self.builder.projection(operand.projection).to_vec();
        projection.push(ProjectionElem::Field(FieldIndex(index as u32), field_ty));
        let projection = self.builder.add_projection(projection);
        Ok(Operand::Copy(Place {
            local: operand.local,
            projection,
        }))
    }

    fn object_temp(&mut self, ty: Ty<'tcx>, value: Rvalue) -> Operand {
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
}
