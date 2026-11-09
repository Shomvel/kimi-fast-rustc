use super::*;
use kfrc_body::TypeOracle;
use kfrc_son::{ComparePredicate, Hole, TraitCall};
use rustc_middle::mir::interpret::Scalar;
use rustc_middle::ty::{TypeVisitableExt, TypingEnv};

impl<'tcx> DefinitionCx<'tcx> {
    pub(super) fn statement(&mut self, statement: &mir::Statement<'tcx>) -> Result<(), String> {
        match &statement.kind {
            mir::StatementKind::Assign(assignment) => {
                let (place, value) = &**assignment;
                let native = place.ty(&self.body.local_decls, self.tcx).ty;
                let ty = self.ty(native)?;
                let value = self.rvalue(value, ty)?;
                let pointer = self.place(*place)?;
                self.store(pointer, value);
            }
            mir::StatementKind::StorageLive(local) | mir::StatementKind::StorageDead(local) => {
                let op = if matches!(statement.kind, mir::StatementKind::StorageLive(_)) {
                    Opcode::SlotLive
                } else {
                    Opcode::SlotDead
                };
                let node = self.graph.add_node(
                    op,
                    self.control,
                    &[self.effect, self.slots[local.as_usize()]],
                    0,
                    ValueType::Tuple(vec![ValueType::Control, ValueType::Effect]),
                );
                self.control = self.projection(node, 0, ValueType::Control);
                self.effect = self.projection(node, 1, ValueType::Effect);
            }
            mir::StatementKind::Intrinsic(intrinsic) => match &**intrinsic {
                mir::NonDivergingIntrinsic::Assume(predicate) => self.assume(predicate)?,
                _ => return Err("statement intrinsic outside definition R subset".into()),
            },
            mir::StatementKind::FakeRead(_)
            | mir::StatementKind::Retag(..)
            | mir::StatementKind::PlaceMention(_)
            | mir::StatementKind::AscribeUserType(..)
            | mir::StatementKind::Coverage(_)
            | mir::StatementKind::ConstEvalCounter
            | mir::StatementKind::BackwardIncompatibleDropHint { .. }
            | mir::StatementKind::Nop => {}
            _ => return Err("statement outside definition R subset".into()),
        }
        Ok(())
    }
    /// Retain the source UB requirement at its original execution point. This
    /// does not query a layout or resolve a target, including in generic branches.
    fn assume(&mut self, predicate: &mir::Operand<'tcx>) -> Result<(), String> {
        if predicate.ty(&self.body.local_decls, self.tcx) != self.tcx.types.bool {
            return Err("R assume predicate is not bool".into());
        }
        let predicate = self.operand(predicate)?;
        let node = self.graph.add_node(
            Opcode::Assume,
            self.control,
            &[self.effect, predicate],
            0,
            ValueType::Tuple(vec![ValueType::Control, ValueType::Effect]),
        );
        self.control = self.projection(node, 0, ValueType::Control);
        self.effect = self.projection(node, 1, ValueType::Effect);
        Ok(())
    }
    fn place(&mut self, place: mir::Place<'tcx>) -> Result<NodeId, String> {
        let mut pointer = self.slots[place.local.as_usize()];
        let mut place_ty = mir::PlaceTy::from_ty(self.body.local_decls[place.local].ty);
        let mut variant = None;
        for projection in place.projection {
            let old_ty = place_ty.ty;
            place_ty = place_ty.projection_ty(self.tcx, projection);
            match projection {
                mir::ProjectionElem::Deref => {
                    let ty = self.ty(old_ty)?;
                    pointer = self.load(pointer, ty);
                }
                mir::ProjectionElem::Downcast(_, index) => variant = Some(index.as_u32()),
                mir::ProjectionElem::Field(field, ty) => {
                    let native = Ty::new_mut_ptr(self.tcx, ty);
                    let pointer_ty = self.ty(native)?;
                    let (op, payload) = if let Some(variant) = variant.take() {
                        let payload = self.graph.variant_fields.len() as u32;
                        self.graph.variant_fields.push((variant, field.as_u32()));
                        (Opcode::VariantFieldAddr, payload)
                    } else {
                        (Opcode::FieldAddr, field.as_u32())
                    };
                    pointer = self.graph.add_node(
                        op,
                        NodeId::NONE,
                        &[pointer],
                        payload,
                        ValueType::Rust(pointer_ty),
                    );
                }
                _ => return Err("place projection outside definition R subset".into()),
            }
        }
        Ok(pointer)
    }
    fn operand(&mut self, operand: &mir::Operand<'tcx>) -> Result<NodeId, String> {
        match operand {
            mir::Operand::Copy(place) | mir::Operand::Move(place) => {
                let ty = self.ty(place.ty(&self.body.local_decls, self.tcx).ty)?;
                let pointer = self.place(*place)?;
                Ok(self.load(pointer, ty))
            }
            mir::Operand::Constant(constant) => {
                let native = constant.const_.ty();
                let ty = self.ty(native)?;
                // Only definition-independent scalar constants are evaluated.
                // Parametric CTFE requires its own future ConstValue hole.
                if constant.const_.has_non_region_param() {
                    return Err("generic constant requires a ConstValue hole".into());
                }
                let value = constant
                    .const_
                    .eval(self.tcx, TypingEnv::fully_monomorphized(), constant.span)
                    .map_err(|_| "generic constant requires a ConstValue hole")?;
                match value {
                    mir::ConstValue::Scalar(Scalar::Int(value)) => {
                        Ok(self.graph.add_constant(value.to_bits(value.size()), ty))
                    }
                    mir::ConstValue::ZeroSized
                        if matches!(self.graph.oracle.shape(ty), TypeShape::Unit) =>
                    {
                        Ok(self.graph.add_node(
                            Opcode::Aggregate,
                            NodeId::NONE,
                            &[],
                            0,
                            ValueType::Rust(ty),
                        ))
                    }
                    _ => Err("non-scalar constant outside definition R subset".into()),
                }
            }
            mir::Operand::RuntimeChecks(checks) => {
                let ty = self.ty(self.tcx.types.bool)?;
                Ok(self
                    .graph
                    .add_constant(u128::from(checks.value(self.tcx.sess)), ty))
            }
        }
    }
    fn rvalue(&mut self, rvalue: &mir::Rvalue<'tcx>, ty: TyId) -> Result<NodeId, String> {
        let value = match rvalue {
            mir::Rvalue::Use(operand) => return self.operand(operand),
            mir::Rvalue::CopyForDeref(place) => return self.operand(&mir::Operand::Copy(*place)),
            mir::Rvalue::Ref(_, _, place) | mir::Rvalue::RawPtr(_, place) => {
                let pointer = self.place(*place)?;
                self.graph.add_node(
                    Opcode::PointerCast,
                    NodeId::NONE,
                    &[pointer],
                    0,
                    ValueType::Rust(ty),
                )
            }
            mir::Rvalue::BinaryOp(op, operands) => {
                let left = self.operand(&operands.0)?;
                let right = self.operand(&operands.1)?;
                let left_ty = self
                    .graph
                    .rust_type(left)
                    .ok_or("binary value lacks Rust type")?;
                let signed = matches!(
                    self.graph.oracle.shape(left_ty),
                    TypeShape::Int { signed: true, .. }
                );
                use mir::BinOp as B;
                let (opcode, payload) = match op {
                    B::Add => (Opcode::Add, 0),
                    B::Sub => (Opcode::Sub, 0),
                    B::Mul => (Opcode::Mul, 0),
                    B::BitAnd => (Opcode::BitAnd, 0),
                    B::BitOr => (Opcode::BitOr, 0),
                    B::BitXor => (Opcode::BitXor, 0),
                    B::Shl => (Opcode::ShiftLeft, 0),
                    B::Shr => (
                        if signed {
                            Opcode::ShiftRightArithmetic
                        } else {
                            Opcode::ShiftRightLogical
                        },
                        0,
                    ),
                    B::Eq => (Opcode::Compare, ComparePredicate::Equal as u32),
                    B::Ne => (Opcode::Compare, ComparePredicate::NotEqual as u32),
                    B::Lt => (
                        Opcode::Compare,
                        if signed {
                            ComparePredicate::SignedLess
                        } else {
                            ComparePredicate::UnsignedLess
                        } as u32,
                    ),
                    B::Le => (
                        Opcode::Compare,
                        if signed {
                            ComparePredicate::SignedLessEqual
                        } else {
                            ComparePredicate::UnsignedLessEqual
                        } as u32,
                    ),
                    B::Gt => (
                        Opcode::Compare,
                        if signed {
                            ComparePredicate::SignedGreater
                        } else {
                            ComparePredicate::UnsignedGreater
                        } as u32,
                    ),
                    B::Ge => (
                        Opcode::Compare,
                        if signed {
                            ComparePredicate::SignedGreaterEqual
                        } else {
                            ComparePredicate::UnsignedGreaterEqual
                        } as u32,
                    ),
                    _ => {
                        return Err(
                            "binary operation needs a definition-level semantic rule".into()
                        );
                    }
                };
                self.graph.add_node(
                    opcode,
                    NodeId::NONE,
                    &[left, right],
                    payload,
                    ValueType::Rust(ty),
                )
            }
            mir::Rvalue::UnaryOp(op, operand) => {
                let value = self.operand(operand)?;
                let op = match op {
                    mir::UnOp::Not => Opcode::BitNot,
                    mir::UnOp::Neg => Opcode::Negate,
                    _ => return Err("generic pointer metadata".into()),
                };
                self.graph
                    .add_node(op, NodeId::NONE, &[value], 0, ValueType::Rust(ty))
            }
            mir::Rvalue::Cast(mir::CastKind::IntToInt, operand, _) => {
                let value = self.operand(operand)?;
                self.graph.add_node(
                    Opcode::IntCast,
                    NodeId::NONE,
                    &[value],
                    0,
                    ValueType::Rust(ty),
                )
            }
            mir::Rvalue::Aggregate(kind, operands) => {
                let values = operands
                    .iter()
                    .map(|operand| self.operand(operand))
                    .collect::<Result<Vec<_>, _>>()?;
                let (op, payload) = match &**kind {
                    mir::AggregateKind::Tuple | mir::AggregateKind::Array(_) => {
                        (Opcode::Aggregate, 0)
                    }
                    mir::AggregateKind::Adt(def, variant, ..)
                        if self.tcx.adt_def(*def).is_enum() =>
                    {
                        (Opcode::MakeVariant, variant.as_u32())
                    }
                    mir::AggregateKind::Adt(def, ..) if self.tcx.adt_def(*def).is_struct() => {
                        (Opcode::Aggregate, 0)
                    }
                    _ => return Err("aggregate outside definition R subset".into()),
                };
                self.graph
                    .add_node(op, NodeId::NONE, &values, payload, ValueType::Rust(ty))
            }
            mir::Rvalue::Discriminant(place) => {
                let value = self.operand(&mir::Operand::Copy(*place))?;
                self.graph.add_node(
                    Opcode::Discriminant,
                    NodeId::NONE,
                    &[value],
                    0,
                    ValueType::Rust(ty),
                )
            }
            mir::Rvalue::Repeat(operand, _) => {
                let value = self.operand(operand)?;
                self.graph.add_node(
                    Opcode::Repeat,
                    NodeId::NONE,
                    &[value],
                    0,
                    ValueType::Rust(ty),
                )
            }
            _ => return Err("rvalue outside definition R subset".into()),
        };
        Ok(value)
    }
    pub(super) fn terminator(&mut self, terminator: &mir::Terminator<'tcx>) -> Result<(), String> {
        match &terminator.kind {
            mir::TerminatorKind::Goto { target }
            | mir::TerminatorKind::FalseEdge {
                real_target: target,
                ..
            }
            | mir::TerminatorKind::FalseUnwind {
                real_target: target,
                ..
            } => self.edge(*target),
            mir::TerminatorKind::Return => {
                let value =
                    self.operand(&mir::Operand::Copy(mir::Place::from(mir::RETURN_PLACE)))?;
                self.graph.add_node(
                    Opcode::Return,
                    self.control,
                    &[self.effect, value],
                    0,
                    ValueType::Control,
                );
            }
            mir::TerminatorKind::Unreachable => {
                self.graph.add_node(
                    Opcode::Unreachable,
                    self.control,
                    &[],
                    0,
                    ValueType::Control,
                );
            }
            mir::TerminatorKind::SwitchInt { discr, targets } => {
                let value = self.operand(discr)?;
                let ty = self
                    .graph
                    .rust_type(value)
                    .ok_or("switch value has no Rust type")?;
                let boolean = self.ty(self.tcx.types.bool)?;
                for (case, target) in targets.iter() {
                    let case = self.graph.add_constant(case, ty);
                    let condition = self.graph.add_node(
                        Opcode::Compare,
                        NodeId::NONE,
                        &[value, case],
                        ComparePredicate::Equal as u32,
                        ValueType::Rust(boolean),
                    );
                    let branch = self.graph.add_node(
                        Opcode::If,
                        self.control,
                        &[condition],
                        0,
                        ValueType::Tuple(vec![ValueType::Control, ValueType::Control]),
                    );
                    self.control = self.projection(branch, 0, ValueType::Control);
                    self.edge(target);
                    self.control = self.projection(branch, 1, ValueType::Control);
                }
                self.edge(targets.otherwise());
            }
            mir::TerminatorKind::Call {
                func,
                args,
                destination,
                target,
                unwind,
                ..
            } => {
                if matches!(unwind, mir::UnwindAction::Cleanup(_)) {
                    return Err("R call cleanup is unsupported".into());
                }
                let native = func.ty(&self.body.local_decls, self.tcx);
                let ty::FnDef(item, generic_args) = *native.kind() else {
                    return Err("R indirect call requires a separate runtime dispatch node".into());
                };
                if self.tcx.intrinsic(item).is_some() {
                    // The intrinsic query establishes compiler identity; an
                    // ordinary user function named `assume` is still a call.
                    if self.tcx.item_name(item).as_str() != "assume" {
                        return Err(
                            "intrinsic call requires the established intrinsic producer".into()
                        );
                    }
                    if args.len() != 1 {
                        return Err("R assume intrinsic requires one predicate".into());
                    }
                    let Some(target) = target else {
                        return Err("R assume intrinsic has no normal continuation".into());
                    };
                    let native_result = destination.ty(&self.body.local_decls, self.tcx).ty;
                    if native_result != self.tcx.types.unit {
                        return Err("R assume intrinsic result is not unit".into());
                    }
                    self.assume(&args[0].node)?;
                    let ty = self.ty(native_result)?;
                    let value = self.graph.add_node(
                        Opcode::Aggregate,
                        NodeId::NONE,
                        &[],
                        0,
                        ValueType::Rust(ty),
                    );
                    let pointer = self.place(*destination)?;
                    self.store(pointer, value);
                    self.edge(*target);
                    return Ok(());
                }
                if InstanceKind::Item(item).requires_caller_location(self.tcx) {
                    return Err("callee track_caller requires a source ABI adapter".into());
                }
                let signature = self
                    .tcx
                    .fn_sig(item)
                    .instantiate(self.tcx, generic_args)
                    .skip_binder();
                if signature.abi == rustc_abi::ExternAbi::RustCall || signature.c_variadic {
                    return Err(
                        "callee tuple spread or variadic ABI requires a source adapter".into(),
                    );
                }
                let destination_ty =
                    self.ty(destination.ty(&self.body.local_decls, self.tcx).ty)?;
                // These are rustc-designated library items, not functions
                // selected by a forgeable display path or spelling.
                let intrinsic = if self
                    .tcx
                    .is_diagnostic_item(rustc_span::sym::mem_size_of, item)
                {
                    Some("size_of")
                } else if self
                    .tcx
                    .is_diagnostic_item(rustc_span::sym::mem_align_of, item)
                {
                    Some("align_of")
                } else if self
                    .tcx
                    .is_diagnostic_item(rustc_span::sym::needs_drop, item)
                {
                    Some("needs_drop")
                } else {
                    None
                };
                let value = if let Some(intrinsic) = intrinsic {
                    if !args.is_empty() {
                        return Err("layout intrinsic has arguments".into());
                    }
                    let queried = self.ty(generic_args.type_at(0))?;
                    if intrinsic == "needs_drop" {
                        let hole = self.graph.add_hole(Hole::NeedsDrop(queried));
                        self.graph.add_node(
                            Opcode::Hole,
                            NodeId::NONE,
                            &[],
                            hole,
                            ValueType::Rust(destination_ty),
                        )
                    } else {
                        let hole = self.graph.add_hole(Hole::SizeAlign(queried));
                        let tuple = self.graph.add_node(
                            Opcode::Hole,
                            NodeId::NONE,
                            &[],
                            hole,
                            ValueType::Tuple(vec![
                                ValueType::Rust(destination_ty),
                                ValueType::Rust(destination_ty),
                            ]),
                        );
                        self.projection(
                            tuple,
                            u32::from(intrinsic == "align_of"),
                            ValueType::Rust(destination_ty),
                        )
                    }
                } else {
                    let values = args
                        .iter()
                        .map(|arg| self.operand(&arg.node))
                        .collect::<Result<Vec<_>, _>>()?;
                    let parameter_types = values
                        .iter()
                        .map(|value| {
                            self.graph
                                .rust_type(*value)
                                .ok_or_else(|| "call argument lacks Rust type".to_owned())
                        })
                        .collect::<Result<Vec<_>, _>>()?;
                    let type_args = generic_args
                        .types()
                        .map(|ty| self.ty(ty))
                        .collect::<Result<Vec<_>, _>>()?;
                    let key = crate::type_identity::name(self.tcx, native);
                    self.resolutions.insert(
                        key.clone(),
                        Resolution {
                            item,
                            args: generic_args,
                        },
                    );
                    let resolve = self.graph.add_hole(Hole::Resolve {
                        item: key,
                        args: type_args,
                    });
                    let payload = self.graph.trait_calls.len() as u32;
                    self.graph.trait_calls.push(TraitCall {
                        resolve,
                        signature: RustSignature {
                            args: parameter_types,
                            ret: destination_ty,
                        },
                    });
                    let mut inputs = vec![self.effect];
                    inputs.extend(values);
                    let call = self.graph.add_node(
                        Opcode::TraitCall,
                        self.control,
                        &inputs,
                        payload,
                        ValueType::Tuple(vec![
                            ValueType::Control,
                            ValueType::Effect,
                            ValueType::Rust(destination_ty),
                        ]),
                    );
                    self.control = self.projection(call, 0, ValueType::Control);
                    self.effect = self.projection(call, 1, ValueType::Effect);
                    self.projection(call, 2, ValueType::Rust(destination_ty))
                };
                if let Some(target) = target {
                    let pointer = self.place(*destination)?;
                    self.store(pointer, value);
                    self.edge(*target);
                } else {
                    self.graph.add_node(
                        Opcode::Unreachable,
                        self.control,
                        &[],
                        0,
                        ValueType::Control,
                    );
                }
            }
            _ => return Err("terminator outside definition R subset".into()),
        }
        Ok(())
    }
}
