//! Intrinsics lowered to the same Typed Body operations as ordinary MIR.

use super::Converter;
use kfrc_body::{
    BinaryOp, BitCountKind, BlockId, Callee, CastKind, Constant, ConstantKind, FunctionAbi,
    FunnelDirection, IntegerIntrinsic, LocalDecl, Mutability, Operand, PassMode, Place,
    ProjectionElem, Rvalue, StatementKind, SwitchTargets, Terminator, UnaryOp, UnwindAction,
};
use rustc_middle::mir;
use rustc_middle::ty::layout::ValidityRequirement;
use rustc_middle::ty::{self, Instance, Ty, TypingEnv};

mod helpers;
mod integer;
mod saturating;
mod select;
mod simd;
mod swap;

/// Shared by monomorphic Body and definition R producers after rustc has
/// authenticated the callee as a compiler intrinsic (not a user-named call).
pub(super) fn integer_operation(name: &str) -> Option<IntegerIntrinsic> {
    Some(match name {
        "ctpop" => IntegerIntrinsic::BitCount {
            kind: BitCountKind::Ones,
            zero_poison: false,
        },
        "ctlz" | "ctlz_nonzero" => IntegerIntrinsic::BitCount {
            kind: BitCountKind::LeadingZeros,
            zero_poison: name == "ctlz_nonzero",
        },
        "cttz" | "cttz_nonzero" => IntegerIntrinsic::BitCount {
            kind: BitCountKind::TrailingZeros,
            zero_poison: name == "cttz_nonzero",
        },
        "bswap" => IntegerIntrinsic::ByteSwap,
        "bitreverse" => IntegerIntrinsic::BitReverse,
        "rotate_left" | "rotate_right" | "unchecked_funnel_shl" | "unchecked_funnel_shr" => {
            IntegerIntrinsic::FunnelShift {
                direction: if matches!(name, "rotate_left" | "unchecked_funnel_shl") {
                    FunnelDirection::Left
                } else {
                    FunnelDirection::Right
                },
                amount_in_range: !matches!(name, "rotate_left" | "rotate_right"),
            }
        }
        "saturating_add" => IntegerIntrinsic::SaturatingAdd,
        "saturating_sub" => IntegerIntrinsic::SaturatingSub,
        _ => return None,
    })
}

impl<'tcx> Converter<'tcx> {
    pub(super) fn intrinsic_call(
        &mut self,
        instance: Instance<'tcx>,
        args: Vec<Operand>,
        mir_args: &[rustc_span::source_map::Spanned<mir::Operand<'tcx>>],
        destination: Place,
        target: Option<BlockId>,
        unwind: UnwindAction,
        source_info: mir::SourceInfo,
    ) -> Result<Terminator, String> {
        let name = self.tcx.item_name(instance.def_id());
        let name = name.as_str();
        if name.starts_with("simd_") {
            self.intrinsic_simd(instance, name, &args, mir_args, destination)?;
            return target
                .map(Terminator::Goto)
                .ok_or_else(|| format!("returning intrinsic `{name}` has no successor"));
        }
        let continuation = |target: Option<BlockId>| {
            target
                .map(Terminator::Goto)
                .ok_or_else(|| format!("returning intrinsic `{name}` has no successor"))
        };
        let assign = |this: &mut Self, value| {
            this.builder
                .push_statement(StatementKind::Assign(destination, value));
        };
        match name {
            "caller_location" => {
                let location = self.caller_location(source_info)?;
                assign(self, Rvalue::Use(location));
            }
            "black_box" => {
                intrinsic_arity(name, &args, 1)?;
                assign(self, Rvalue::Opaque(args[0]));
            }
            "is_val_statically_known" => {
                intrinsic_arity(name, &args, 1)?;
                // This is an optimization hint, not a constant-evaluation
                // test. Its upstream Rust body returns false, and callers
                // may not rely on a particular answer. Keep evaluation of
                // the operand attached to the effect chain before returning
                // that conservative answer, even when the result is unused.
                self.intrinsic_temp(instance.args.type_at(0), Rvalue::Opaque(args[0]));
                let unknown = self.intrinsic_constant(self.tcx.types.bool, 0);
                assign(self, Rvalue::Use(unknown));
            }
            "const_eval_select" => {
                // rustc selects the runtime function and expands its tuple
                // before optimized_mir is exposed to codegen, independently
                // of mir-opt-level. Never choose a const branch here.
                return Err(
                    "const_eval_select reached codegen before rustc's runtime remapping".into(),
                );
            }
            "likely" | "unlikely" => {
                intrinsic_arity(name, &args, 1)?;
                assign(self, Rvalue::Use(args[0]));
            }
            "select_unpredictable" => {
                intrinsic_arity(name, &args, 3)?;
                return self.intrinsic_select(&args, destination, target);
            }
            "cold_path" => intrinsic_arity(name, &args, 0)?,
            "unreachable" => return Ok(Terminator::Unreachable),
            "abort" => return Ok(self.intrinsic_libcall("abort", Vec::new(), None, None, unwind)),
            "assume" => {
                intrinsic_arity(name, &args, 1)?;
                self.builder.push_statement(StatementKind::Assume(args[0]));
                let ty = self.ty(self.tcx.types.unit);
                let unit = Operand::Constant(self.builder.add_constant(Constant {
                    ty,
                    kind: ConstantKind::ZeroSized,
                }));
                assign(self, Rvalue::Use(unit));
            }
            "size_of" | "align_of" | "min_align_of" | "needs_drop" => {
                intrinsic_arity(name, &args, 0)?;
                let ty = instance.args.type_at(0);
                let value = if name == "needs_drop" {
                    u128::from(ty.needs_drop(self.tcx, TypingEnv::fully_monomorphized()))
                } else {
                    let layout = self.intrinsic_layout(ty)?;
                    if name == "size_of" {
                        u128::from(layout.0)
                    } else {
                        u128::from(layout.1)
                    }
                };
                let value_ty = if name == "needs_drop" {
                    self.tcx.types.bool
                } else {
                    self.tcx.types.usize
                };
                let value = self.intrinsic_constant(value_ty, value);
                assign(self, Rvalue::Use(value));
            }
            "size_of_val" | "align_of_val" | "min_align_of_val" => {
                intrinsic_arity(name, &args, 1)?;
                let ty = instance.args.type_at(0);
                if matches!(ty.kind(), ty::Dynamic(..)) {
                    let (_, vtable) = self.object_parts(args[0], Ty::new_imm_ptr(self.tcx, ty))?;
                    let slot = if name == "size_of_val" {
                        ty::COMMON_VTABLE_ENTRIES_SIZE
                    } else {
                        ty::COMMON_VTABLE_ENTRIES_ALIGN
                    };
                    let value = self.vtable_slot(vtable, slot)?;
                    assign(self, Rvalue::Use(value));
                    return continuation(target);
                }
                let value = if ty.is_sized(self.tcx, TypingEnv::fully_monomorphized()) {
                    let (size, align) = self.intrinsic_layout(ty)?;
                    self.intrinsic_constant(
                        self.tcx.types.usize,
                        u128::from(if name == "size_of_val" { size } else { align }),
                    )
                } else {
                    let (element_size, align) = match ty.kind() {
                        ty::Slice(element) => self.intrinsic_layout(*element)?,
                        ty::Str => (1, 1),
                        _ => return Err(format!("intrinsic `{name}` on unsized `{ty}`")),
                    };
                    if name != "size_of_val" {
                        self.intrinsic_constant(self.tcx.types.usize, u128::from(align))
                    } else {
                        let meta = self.intrinsic_temp(
                            self.tcx.types.usize,
                            Rvalue::UnaryOp(UnaryOp::PtrMetadata, args[0]),
                        );
                        let scale =
                            self.intrinsic_constant(self.tcx.types.usize, u128::from(element_size));
                        self.intrinsic_temp(
                            self.tcx.types.usize,
                            Rvalue::BinaryOp(BinaryOp::Mul, meta, scale),
                        )
                    }
                };
                assign(self, Rvalue::Use(value));
            }
            "vtable_size" | "vtable_align" => {
                intrinsic_arity(name, &args, 1)?;
                let slot = if name == "vtable_size" {
                    ty::COMMON_VTABLE_ENTRIES_SIZE
                } else {
                    ty::COMMON_VTABLE_ENTRIES_ALIGN
                };
                let value = self.vtable_slot(args[0], slot)?;
                assign(self, Rvalue::Use(value));
            }
            "arith_offset" | "offset" => {
                intrinsic_arity(name, &args, 2)?;
                // Keep typed element indexing until L chooses its stride.
                // Integerizing here loses this fact and creates casts, a
                // size constant and arithmetic that later passes must undo.
                assign(self, Rvalue::BinaryOp(BinaryOp::Offset, args[0], args[1]));
            }
            "ptr_offset_from" | "ptr_offset_from_unsigned" => {
                intrinsic_arity(name, &args, 2)?;
                let result_ty = if name.ends_with("unsigned") {
                    self.tcx.types.usize
                } else {
                    self.tcx.types.isize
                };
                let element = instance.args.type_at(0);
                let size = self.intrinsic_layout(element)?.0;
                if size == 0 {
                    return Err("ptr_offset_from requires a nonzero-sized pointee".into());
                }
                let left = self.intrinsic_cast(args[0], result_ty);
                let right = self.intrinsic_cast(args[1], result_ty);
                // The unsigned intrinsic promises a nonnegative distance.
                // Signed pointer subtraction has no NSW promise in rustc.
                let subtraction = if name.ends_with("unsigned") {
                    BinaryOp::SubUnchecked
                } else {
                    BinaryOp::Sub
                };
                let bytes =
                    self.intrinsic_temp(result_ty, Rvalue::BinaryOp(subtraction, left, right));
                let size = self.intrinsic_constant(result_ty, u128::from(size));
                assign(self, Rvalue::BinaryOp(BinaryOp::DivExact, bytes, size));
            }
            "ptr_mask" => {
                intrinsic_arity(name, &args, 2)?;
                let ptr_ty = self.intrinsic_result_ty(instance);
                let value = self.intrinsic_cast(args[0], self.tcx.types.usize);
                let mask = self.intrinsic_cast(args[1], self.tcx.types.usize);
                let bits = self.intrinsic_temp(
                    self.tcx.types.usize,
                    Rvalue::BinaryOp(BinaryOp::BitAnd, value, mask),
                );
                let target_ty = self.ty(ptr_ty);
                assign(self, Rvalue::Cast(CastKind::IntToInt, bits, target_ty));
            }
            "read_via_copy" => {
                intrinsic_arity(name, &args, 1)?;
                let source = self.intrinsic_deref(args[0], instance.args.type_at(0));
                assign(self, Rvalue::Use(Operand::Copy(source)));
            }
            "write_via_move" => {
                intrinsic_arity(name, &args, 2)?;
                let place = self.intrinsic_deref(args[0], instance.args.type_at(0));
                self.builder
                    .push_statement(StatementKind::Assign(place, Rvalue::Use(args[1])));
            }
            "discriminant_value" => {
                intrinsic_arity(name, &args, 1)?;
                let ty = instance.args.type_at(0);
                let place = self.intrinsic_deref(args[0], ty);
                let value = self.discriminant_rvalue(ty, place);
                assign(self, value);
            }
            "typed_swap_nonoverlapping" => {
                intrinsic_arity(name, &args, 2)?;
                self.intrinsic_typed_swap(instance.args.type_at(0), args[0], args[1])?;
            }
            "copy" | "copy_nonoverlapping" => {
                intrinsic_arity(name, &args, 3)?;
                let bytes = self.intrinsic_scale(args[2], instance.args.type_at(0))?;
                return Ok(self.intrinsic_libcall(
                    if name == "copy" { "memmove" } else { "memcpy" },
                    vec![args[1], args[0], bytes],
                    Some(self.tcx.types.usize),
                    target,
                    unwind,
                ));
            }
            "write_bytes" => {
                intrinsic_arity(name, &args, 3)?;
                let value = self.intrinsic_cast(args[1], self.tcx.types.i32);
                let bytes = self.intrinsic_scale(args[2], instance.args.type_at(0))?;
                return Ok(self.intrinsic_libcall(
                    "memset",
                    vec![args[0], value, bytes],
                    Some(self.tcx.types.usize),
                    target,
                    unwind,
                ));
            }
            "compare_bytes" => {
                intrinsic_arity(name, &args, 3)?;
                // MIR may move each operand. The length switch, the byte
                // reads and the memcmp call all use them, so consume each once
                // into a temporary; a repeated Move reads an empty slot.
                let byte_pointer = Ty::new_imm_ptr(self.tcx, self.tcx.types.u8);
                let left = self.intrinsic_temp(byte_pointer, Rvalue::Use(args[0]));
                let right = self.intrinsic_temp(byte_pointer, Rvalue::Use(args[1]));
                let length = self.intrinsic_temp(self.tcx.types.usize, Rvalue::Use(args[2]));
                let call = self.intrinsic_libcall(
                    "memcmp",
                    vec![left, right, length],
                    Some(self.tcx.types.i32),
                    target,
                    unwind,
                );
                let Terminator::Call {
                    callee,
                    args,
                    target,
                    unwind,
                    ..
                } = call
                else {
                    unreachable!()
                };
                let Some(continuation) = target else {
                    return Err("compare_bytes has no successor".into());
                };
                // Char patterns compare one encoded byte; most slices are
                // ASCII. A one-byte comparison is a subtraction of the bytes,
                // a valid memcmp ordering; other lengths call memcmp. The
                // casts only retype the pointers; bytes are read in the branch.
                let byte = self.tcx.types.u8;
                let left = self.intrinsic_deref(left, byte);
                let right = self.intrinsic_deref(right, byte);
                let int = self.ty(self.tcx.types.i32);
                let mut widen = |this: &mut Self, place: Place| {
                    let local = this.builder.add_local(LocalDecl {
                        ty: int,
                        mutability: Mutability::Mut,
                        name: None,
                    });
                    (
                        local,
                        StatementKind::Assign(
                            Place::local(local),
                            Rvalue::Cast(CastKind::IntToInt, Operand::Copy(place), int),
                        ),
                    )
                };
                let (left_byte, load_left) = widen(self, left);
                let (right_byte, load_right) = widen(self, right);
                let single = BlockId((self.original_blocks + self.pending_blocks.len()) as u32);
                self.pending_blocks.push((
                    vec![
                        load_left,
                        load_right,
                        StatementKind::Assign(
                            destination,
                            Rvalue::BinaryOp(
                                BinaryOp::Sub,
                                Operand::Copy(Place::local(left_byte)),
                                Operand::Copy(Place::local(right_byte)),
                            ),
                        ),
                    ],
                    Terminator::Goto(continuation),
                ));
                let general = BlockId((self.original_blocks + self.pending_blocks.len()) as u32);
                self.pending_blocks.push((
                    Vec::new(),
                    Terminator::Call {
                        callee,
                        args,
                        destination,
                        target,
                        unwind,
                    },
                ));
                return Ok(Terminator::SwitchInt {
                    discriminant: length,
                    targets: SwitchTargets {
                        cases: vec![(1, single)],
                        otherwise: general,
                    },
                });
            }
            "raw_eq" => {
                intrinsic_arity(name, &args, 2)?;
                let size = self.intrinsic_layout(instance.args.type_at(0))?.0;
                if size == 0 {
                    let value = self.intrinsic_constant(self.tcx.types.bool, 1);
                    assign(self, Rvalue::Use(value));
                } else {
                    let target = target.ok_or("raw_eq has no successor")?;
                    let bytes = self.intrinsic_constant(self.tcx.types.usize, u128::from(size));
                    // Equality only: `bcmp` returns zero exactly when the bytes
                    // match, and the backend expands small constant sizes.
                    let mut call = self.intrinsic_libcall(
                        "bcmp",
                        vec![args[0], args[1], bytes],
                        Some(self.tcx.types.i32),
                        None,
                        unwind,
                    );
                    let Terminator::Call {
                        destination: result,
                        target: call_target,
                        ..
                    } = &mut call
                    else {
                        unreachable!()
                    };
                    let zero = self.intrinsic_constant(self.tcx.types.i32, 0);
                    let next = BlockId((self.original_blocks + self.pending_blocks.len()) as u32);
                    self.pending_blocks.push((
                        vec![StatementKind::Assign(
                            destination,
                            Rvalue::BinaryOp(BinaryOp::Eq, Operand::Copy(*result), zero),
                        )],
                        Terminator::Goto(target),
                    ));
                    *call_target = Some(next);
                    return Ok(call);
                }
            }
            "disjoint_bitor" => {
                intrinsic_arity(name, &args, 2)?;
                assign(
                    self,
                    Rvalue::BinaryOp(BinaryOp::BitOrDisjoint, args[0], args[1]),
                );
            }
            "saturating_add" | "saturating_sub" => {
                let result = self.intrinsic_saturating(name, instance.args.type_at(0), &args)?;
                assign(self, Rvalue::Use(result));
            }
            "exact_div" | "wrapping_add" | "wrapping_sub" | "wrapping_mul" | "unchecked_add"
            | "unchecked_sub" | "unchecked_mul" | "unchecked_shl" | "unchecked_shr"
            | "add_with_overflow" | "sub_with_overflow" | "mul_with_overflow" => {
                intrinsic_arity(name, &args, 2)?;
                let op = match name {
                    "exact_div" => BinaryOp::DivExact,
                    "wrapping_add" => BinaryOp::Add,
                    "wrapping_sub" => BinaryOp::Sub,
                    "wrapping_mul" => BinaryOp::Mul,
                    "unchecked_add" => BinaryOp::AddUnchecked,
                    "unchecked_sub" => BinaryOp::SubUnchecked,
                    "unchecked_mul" => BinaryOp::MulUnchecked,
                    "unchecked_shl" => BinaryOp::ShlUnchecked,
                    "unchecked_shr" => BinaryOp::ShrUnchecked,
                    "add_with_overflow" => BinaryOp::AddWithOverflow,
                    "sub_with_overflow" => BinaryOp::SubWithOverflow,
                    _ => BinaryOp::MulWithOverflow,
                };
                assign(self, Rvalue::BinaryOp(op, args[0], args[1]));
            }
            "rotate_left" | "rotate_right" | "unchecked_funnel_shl" | "unchecked_funnel_shr" => {
                let result = self.intrinsic_funnel(name, instance.args.type_at(0), &args)?;
                assign(self, Rvalue::Use(result));
            }
            "bswap" | "bitreverse" | "ctpop" | "cttz" | "cttz_nonzero" | "ctlz"
            | "ctlz_nonzero" => {
                let result = self.intrinsic_bits(name, instance.args.type_at(0), &args)?;
                assign(self, Rvalue::Use(result));
            }
            "assert_inhabited" | "assert_zero_valid" | "assert_mem_uninitialized_valid" => {
                intrinsic_arity(name, &args, 0)?;
                let requirement = match name {
                    "assert_inhabited" => ValidityRequirement::Inhabited,
                    "assert_zero_valid" => ValidityRequirement::Zero,
                    _ => ValidityRequirement::UninitMitigated0x01Fill,
                };
                let ty = instance.args.type_at(0);
                let valid = self
                    .tcx
                    .check_validity_requirement((
                        requirement,
                        TypingEnv::fully_monomorphized().as_query_input(ty),
                    ))
                    .map_err(|error| format!("validity requirement for `{ty}`: {error:?}"))?;
                if !valid {
                    return Err(format!(
                        "intrinsic `{name}` for invalid type `{ty}` requires a panic diagnostic"
                    ));
                }
            }
            "transmute" => {
                intrinsic_arity(name, &args, 1)?;
                let target_ty = self.ty(instance.args.type_at(1));
                assign(self, Rvalue::Cast(CastKind::Transmute, args[0], target_ty));
            }
            _ => return Err(format!("intrinsic `{name}` is not implemented")),
        }
        continuation(target)
    }
}

fn intrinsic_arity(name: &str, args: &[Operand], count: usize) -> Result<(), String> {
    if args.len() == count {
        Ok(())
    } else {
        Err(format!(
            "intrinsic `{name}` expects {count} operands, got {}",
            args.len()
        ))
    }
}
