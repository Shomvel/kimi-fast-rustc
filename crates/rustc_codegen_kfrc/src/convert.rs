//! rustc MIR → kfrc Typed Body (seam 2 of `docs/plan/poc/architecture.md`).
//!
//! The MIR of a mono item is monomorphized and translated one-to-one into
//! the Body contract: blocks keep their indices, places keep their
//! projections, and call targets are resolved to concrete instances whose
//! linker symbols and `PassMode`s come from rustc (so mixed linking with
//! rustc-compiled std is ABI-correct). Types are recorded with rustc's
//! layouts into the Body's `RecordedTypes`; the SoN never sees `Ty<'tcx>`.
//!
//! This converter is throwaway PoC code: what it cannot express is a gap in
//! the Body contract, recorded rather than worked around.

use std::collections::HashMap;

use kfrc_body::{
    AggregateKind, AssertKind, BinaryOp, BlockId, Body, BodyBuilder, BorrowKind, Callee, CastKind,
    Constant, ConstantKind, FieldIndex, FunctionAbi, IntegerWidth, LocalDecl, LocalId, Mutability,
    Operand, PassMode, Place, ProjectionElem, RecordedTypes, Rvalue, SizeAlign, StatementKind,
    SwitchTargets, Terminator, TyId, TypeShape, UnaryOp, UnwindAction, VariantIndex,
};
use rustc_middle::mir::{self, interpret::Scalar};
use rustc_middle::ty::{self, Instance, InstanceKind, Ty, TyCtxt, TypingEnv};
use rustc_target::callconv;

pub fn convert_instance<'tcx>(tcx: TyCtxt<'tcx>, instance: Instance<'tcx>) -> Result<Body, String> {
    let mir = tcx.instance_mir(instance.def);
    let abi = function_abi(tcx, instance)?;
    if abi.args.len() != mir.arg_count {
        return Err(format!(
            "ABI has {} arguments but MIR has {} (implicit `#[track_caller]` location or `rust-call` tupling)",
            abi.args.len(),
            mir.arg_count
        ));
    }
    let mut converter = Converter {
        tcx,
        instance,
        types: HashMap::new(),
        builder: BodyBuilder::new(tcx.symbol_name(instance).name.to_string(), abi, RecordedTypes::new()),
    };
    for (local, declaration) in mir.local_decls.iter_enumerated() {
        let ty = converter.ty(converter.monomorphize(declaration.ty));
        let mutability = match declaration.mutability {
            mir::Mutability::Mut => Mutability::Mut,
            mir::Mutability::Not => Mutability::Not,
        };
        let name = mir
            .var_debug_info
            .iter()
            .find(|info| matches!(info.value, mir::VarDebugInfoContents::Place(place) if place.local == local && place.projection.is_empty()))
            .map(|info| info.name.to_string());
        converter.builder.add_local(LocalDecl { ty, mutability, name });
    }
    converter.builder.set_arg_count(mir.arg_count);
    for (_, block) in mir.basic_blocks.iter_enumerated() {
        for statement in &block.statements {
            if let Some(statement) = converter.statement(statement)? {
                converter.builder.push_statement(statement);
            }
        }
        let terminator = converter.terminator(block.terminator())?;
        converter.builder.finish_block(terminator, block.is_cleanup);
    }
    Ok(converter.builder.finish())
}

fn function_abi<'tcx>(tcx: TyCtxt<'tcx>, instance: Instance<'tcx>) -> Result<FunctionAbi, String> {
    let fn_abi = tcx
        .fn_abi_of_instance(TypingEnv::fully_monomorphized().as_query_input((instance, ty::List::empty())))
        .map_err(|error| format!("no ABI for `{instance}`: {error:?}"))?;
    if fn_abi.c_variadic {
        return Err(format!("C-variadic function `{instance}`"));
    }
    Ok(FunctionAbi {
        args: fn_abi.args.iter().map(|arg| pass_mode(&arg.mode)).collect(),
        ret: pass_mode(&fn_abi.ret.mode),
        c_variadic: false,
    })
}

fn pass_mode(mode: &callconv::PassMode) -> PassMode {
    match mode {
        callconv::PassMode::Ignore => PassMode::Ignore,
        callconv::PassMode::Direct(_) => PassMode::Direct,
        callconv::PassMode::Pair(..) => PassMode::Pair,
        callconv::PassMode::Cast { .. } => PassMode::Cast,
        callconv::PassMode::Indirect { .. } => PassMode::Indirect,
    }
}

struct Converter<'tcx> {
    tcx: TyCtxt<'tcx>,
    instance: Instance<'tcx>,
    types: HashMap<Ty<'tcx>, TyId>,
    builder: BodyBuilder,
}

impl<'tcx> Converter<'tcx> {
    fn monomorphize<T>(&self, value: T) -> T
    where
        T: ty::TypeFoldable<TyCtxt<'tcx>>,
    {
        self.instance.instantiate_mir_and_normalize_erasing_regions(
            self.tcx,
            TypingEnv::fully_monomorphized(),
            ty::EarlyBinder::bind(value),
        )
    }

    fn ty(&mut self, ty: Ty<'tcx>) -> TyId {
        if let Some(id) = self.types.get(&ty) {
            return *id;
        }
        let shape = match ty.kind() {
            ty::Bool => TypeShape::Bool,
            ty::Char => TypeShape::Char,
            ty::Int(int) => TypeShape::Int { width: integer_width(int.bit_width()), signed: true },
            ty::Uint(uint) => TypeShape::Int { width: integer_width(uint.bit_width()), signed: false },
            ty::Float(float) => TypeShape::Float { bits: float.bit_width() as u8 },
            ty::Tuple(elements) if elements.is_empty() => TypeShape::Unit,
            ty::Tuple(elements) => TypeShape::Tuple(elements.iter().map(|element| self.ty(element)).collect()),
            ty::Never => TypeShape::Never,
            ty::Ref(_, pointee, mutability) => TypeShape::Reference {
                mutability: convert_mutability(*mutability),
                pointee: self.ty(*pointee),
            },
            ty::RawPtr(pointee, mutability) => TypeShape::RawPointer {
                mutability: convert_mutability(*mutability),
                pointee: self.ty(*pointee),
            },
            ty::Adt(..) => TypeShape::Adt { name: ty.to_string() },
            ty::FnDef(..) => TypeShape::FunctionItem { name: ty.to_string() },
            _ => TypeShape::Other { name: ty.to_string() },
        };
        let layout = self
            .tcx
            .layout_of(TypingEnv::fully_monomorphized().as_query_input(ty))
            .ok()
            .filter(|layout| layout.is_sized())
            .map(|layout| SizeAlign {
                size: layout.size.bytes(),
                align: layout.align.abi.bytes(),
            });
        let id = self.builder.types().intern(shape, layout);
        self.types.insert(ty, id);
        id
    }

    fn place(&mut self, place: mir::Place<'tcx>) -> Result<Place, String> {
        let mut elements = Vec::new();
        for element in place.projection {
            elements.push(match element {
                mir::ProjectionElem::Field(field, ty) => {
                    let ty = self.ty(self.monomorphize(ty));
                    ProjectionElem::Field(FieldIndex(field.as_u32()), ty)
                }
                mir::ProjectionElem::Deref => ProjectionElem::Deref,
                mir::ProjectionElem::Index(local) => ProjectionElem::Index(LocalId(local.as_u32())),
                mir::ProjectionElem::ConstantIndex { offset, min_length, from_end } => {
                    ProjectionElem::ConstantIndex { offset, min_length, from_end }
                }
                mir::ProjectionElem::Subslice { from, to, from_end } => ProjectionElem::Subslice { from, to, from_end },
                mir::ProjectionElem::Downcast(_, variant) => ProjectionElem::Downcast(VariantIndex(variant.as_u32())),
                other => return Err(format!("place projection {other:?}")),
            });
        }
        Ok(Place {
            local: LocalId(place.local.as_u32()),
            projection: self.builder.add_projection(elements),
        })
    }

    fn operand(&mut self, operand: &mir::Operand<'tcx>) -> Result<Operand, String> {
        match operand {
            mir::Operand::Copy(place) => Ok(Operand::Copy(self.place(*place)?)),
            mir::Operand::Move(place) => Ok(Operand::Move(self.place(*place)?)),
            mir::Operand::Constant(constant) => {
                let value = self.monomorphize(constant.const_);
                let ty = value.ty();
                let kind = if let ty::FnDef(def_id, args) = *ty.kind() {
                    ConstantKind::Function(self.callee(def_id, args, constant.span)?)
                } else {
                    let evaluated = value
                        .eval(self.tcx, TypingEnv::fully_monomorphized(), constant.span)
                        .map_err(|_| format!("constant of type `{ty}` failed to evaluate"))?;
                    match evaluated {
                        mir::ConstValue::Scalar(Scalar::Int(int)) => ConstantKind::Scalar(int.to_bits(int.size())),
                        mir::ConstValue::ZeroSized => ConstantKind::ZeroSized,
                        other => return Err(format!("constant `{other:?}` of type `{ty}` (pointers and aggregates arrive in P1)")),
                    }
                };
                let ty = self.ty(ty);
                Ok(Operand::Constant(self.builder.add_constant(Constant { ty, kind })))
            }
            mir::Operand::RuntimeChecks(_) => Err("`RuntimeChecks` operand".to_owned()),
        }
    }

    fn callee(&mut self, def_id: rustc_span::def_id::DefId, args: ty::GenericArgsRef<'tcx>, span: rustc_span::Span) -> Result<Callee, String> {
        let instance = Instance::expect_resolve(self.tcx, TypingEnv::fully_monomorphized(), def_id, args, span);
        match instance.def {
            InstanceKind::Intrinsic(def_id) => Ok(Callee::Intrinsic {
                name: self.tcx.item_name(def_id).to_string(),
            }),
            InstanceKind::Virtual(..) => Err(format!("virtual call `{instance}` (dyn arrives as an extension goal)")),
            _ => {
                let symbol = self.tcx.symbol_name(instance).name.to_string();
                if symbol.starts_with("llvm.") {
                    return Err(format!("LLVM intrinsic `{symbol}`"));
                }
                let abi = function_abi(self.tcx, instance)?;
                Ok(Callee::Function { symbol, abi })
            }
        }
    }

    fn statement(&mut self, statement: &mir::Statement<'tcx>) -> Result<Option<StatementKind>, String> {
        Ok(Some(match &statement.kind {
            mir::StatementKind::Assign(assignment) => {
                let (place, rvalue) = &**assignment;
                StatementKind::Assign(self.place(*place)?, self.rvalue(rvalue)?)
            }
            mir::StatementKind::StorageLive(local) => StatementKind::StorageLive(LocalId(local.as_u32())),
            mir::StatementKind::StorageDead(local) => StatementKind::StorageDead(LocalId(local.as_u32())),
            mir::StatementKind::SetDiscriminant { place, variant_index } => {
                StatementKind::SetDiscriminant(self.place(**place)?, VariantIndex(variant_index.as_u32()))
            }
            mir::StatementKind::Intrinsic(intrinsic) => return Err(format!("statement intrinsic {intrinsic:?}")),
            // Checker-only and debugging statements carry no runtime semantics.
            mir::StatementKind::FakeRead(_)
            | mir::StatementKind::Retag(..)
            | mir::StatementKind::PlaceMention(_)
            | mir::StatementKind::AscribeUserType(..)
            | mir::StatementKind::Coverage(_)
            | mir::StatementKind::ConstEvalCounter
            | mir::StatementKind::BackwardIncompatibleDropHint { .. }
            | mir::StatementKind::Nop => return Ok(None),
        }))
    }

    fn rvalue(&mut self, rvalue: &mir::Rvalue<'tcx>) -> Result<Rvalue, String> {
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
            mir::Rvalue::Cast(kind, operand, ty) => {
                let kind = match kind {
                    mir::CastKind::IntToInt => CastKind::IntToInt,
                    mir::CastKind::IntToFloat => CastKind::IntToFloat,
                    mir::CastKind::FloatToInt => CastKind::FloatToInt,
                    mir::CastKind::FloatToFloat => CastKind::FloatToFloat,
                    mir::CastKind::PtrToPtr => CastKind::PtrToPtr,
                    mir::CastKind::Transmute => CastKind::Transmute,
                    _ => CastKind::Other,
                };
                let ty = self.ty(self.monomorphize(*ty));
                Rvalue::Cast(kind, self.operand(operand)?, ty)
            }
            mir::Rvalue::Ref(_, kind, place) => {
                let kind = match kind {
                    mir::BorrowKind::Shared => BorrowKind::Shared,
                    mir::BorrowKind::Fake(_) => BorrowKind::Fake,
                    mir::BorrowKind::Mut { .. } => BorrowKind::Mut,
                };
                Rvalue::Ref(kind, self.place(*place)?)
            }
            mir::Rvalue::RawPtr(kind, place) => {
                let mutability = if kind.to_mutbl_lossy() == mir::Mutability::Mut { Mutability::Mut } else { Mutability::Not };
                Rvalue::RawPointer(mutability, self.place(*place)?)
            }
            mir::Rvalue::Aggregate(kind, operands) => {
                let kind = match &**kind {
                    mir::AggregateKind::Tuple => AggregateKind::Tuple,
                    mir::AggregateKind::Array(ty) => AggregateKind::Array(self.ty(self.monomorphize(*ty))),
                    mir::AggregateKind::Adt(def_id, variant, args, _, _) => {
                        let ty = self.monomorphize(self.tcx.type_of(*def_id).instantiate(self.tcx, args));
                        AggregateKind::Adt { ty: self.ty(ty), variant: VariantIndex(variant.as_u32()) }
                    }
                    mir::AggregateKind::Closure(..) => AggregateKind::Other,
                    _ => AggregateKind::Other,
                };
                let mut converted = Vec::with_capacity(operands.len());
                for operand in operands {
                    converted.push(self.operand(operand)?);
                }
                Rvalue::Aggregate(kind, converted)
            }
            mir::Rvalue::Discriminant(place) => Rvalue::Discriminant(self.place(*place)?),
            mir::Rvalue::Repeat(..) => Rvalue::Unsupported("array repeat expression".to_owned()),
            mir::Rvalue::ThreadLocalRef(_) => Rvalue::Unsupported("thread-local reference".to_owned()),
            mir::Rvalue::ShallowInitBox(..) => Rvalue::Unsupported("ShallowInitBox".to_owned()),
            mir::Rvalue::CopyForDeref(_) => Rvalue::Unsupported("CopyForDeref".to_owned()),
            mir::Rvalue::WrapUnsafeBinder(..) => Rvalue::Unsupported("unsafe binder".to_owned()),
        })
    }

    fn unwind(&self, action: &mir::UnwindAction) -> UnwindAction {
        match action {
            mir::UnwindAction::Continue => UnwindAction::Continue,
            mir::UnwindAction::Unreachable => UnwindAction::Unreachable,
            mir::UnwindAction::Terminate(_) => UnwindAction::Terminate,
            mir::UnwindAction::Cleanup(block) => UnwindAction::Cleanup(BlockId(block.as_u32())),
        }
    }

    fn terminator(&mut self, terminator: &mir::Terminator<'tcx>) -> Result<Terminator, String> {
        let block = |target: &mir::BasicBlock| BlockId(target.as_u32());
        Ok(match &terminator.kind {
            mir::TerminatorKind::Goto { target } => Terminator::Goto(block(target)),
            mir::TerminatorKind::FalseEdge { real_target, .. } | mir::TerminatorKind::FalseUnwind { real_target, .. } => {
                Terminator::Goto(block(real_target))
            }
            mir::TerminatorKind::SwitchInt { discr, targets } => Terminator::SwitchInt {
                discriminant: self.operand(discr)?,
                targets: SwitchTargets {
                    cases: targets.iter().map(|(value, target)| (value, block(&target))).collect(),
                    otherwise: block(&targets.otherwise()),
                },
            },
            mir::TerminatorKind::Return => Terminator::Return,
            mir::TerminatorKind::Unreachable => Terminator::Unreachable,
            mir::TerminatorKind::UnwindResume => Terminator::UnwindResume,
            mir::TerminatorKind::UnwindTerminate(_) => Terminator::UnwindTerminate,
            mir::TerminatorKind::Drop { place, target, unwind, .. } => Terminator::Drop {
                place: self.place(*place)?,
                target: block(target),
                unwind: self.unwind(unwind),
            },
            mir::TerminatorKind::Call { func, args, destination, target, unwind, .. } => {
                let callee = self.operand(func)?;
                let mut converted = Vec::with_capacity(args.len());
                for arg in args {
                    converted.push(self.operand(&arg.node)?);
                }
                if let Operand::Constant(id) = callee
                    && let Constant { kind: ConstantKind::Function(Callee::Function { abi, symbol }), .. } =
                        self.builder_constant(id)
                    && abi.args.len() != converted.len()
                {
                    return Err(format!(
                        "call to `{symbol}` passes {} arguments but its ABI has {} (implicit `#[track_caller]` location)",
                        converted.len(),
                        abi.args.len()
                    ));
                }
                Terminator::Call {
                    callee,
                    args: converted,
                    destination: self.place(*destination)?,
                    target: target.as_ref().map(block),
                    unwind: self.unwind(unwind),
                }
            }
            mir::TerminatorKind::Assert { cond, expected, msg, target, unwind } => Terminator::Assert {
                condition: self.operand(cond)?,
                expected: *expected,
                message: match &**msg {
                    mir::AssertKind::Overflow(op, ..) => AssertKind::Overflow(binary_op(*op)),
                    mir::AssertKind::OverflowNeg(_) => AssertKind::OverflowNeg,
                    mir::AssertKind::DivisionByZero(_) => AssertKind::DivisionByZero,
                    mir::AssertKind::RemainderByZero(_) => AssertKind::RemainderByZero,
                    mir::AssertKind::BoundsCheck { .. } => AssertKind::BoundsCheck,
                    other => AssertKind::Other(format!("{other:?}")),
                },
                target: block(target),
                unwind: self.unwind(unwind),
            },
            other => return Err(format!("terminator {:?}", other.name())),
        })
    }

    fn builder_constant(&self, id: kfrc_body::ConstantId) -> Constant {
        self.builder.constant(id)
    }
}

fn integer_width(bits: Option<u64>) -> IntegerWidth {
    match bits {
        Some(8) => IntegerWidth::W8,
        Some(16) => IntegerWidth::W16,
        Some(32) => IntegerWidth::W32,
        Some(64) => IntegerWidth::W64,
        Some(128) => IntegerWidth::W128,
        _ => IntegerWidth::Pointer,
    }
}

fn convert_mutability(mutability: mir::Mutability) -> Mutability {
    match mutability {
        mir::Mutability::Mut => Mutability::Mut,
        mir::Mutability::Not => Mutability::Not,
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
