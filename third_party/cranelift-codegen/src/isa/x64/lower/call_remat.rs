//! kfrc: opt-in, bounded scalar constants at ordinary direct-call use points.

use super::{Inst, OperandSize};
use crate::ir::{ArgumentPurpose, DataFlowGraph, InstructionData, Opcode, Value, types};
use crate::machinst::isle::{ValueRegs, ValueRegsVec, ValueSlice};
use crate::machinst::{ABIArg, Lower, Sig};

const MAX_PER_CALL: u8 = 4;
const MAX_PER_FUNCTION: u8 = 64;

pub(super) fn enabled() -> bool {
    #[cfg(feature = "std")]
    {
        static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        *ENABLED.get_or_init(|| {
            std::env::var_os("KFRC_CLIF_CALL_CONST_REMAT").is_some_and(|value| value == "1")
        })
    }
    #[cfg(not(feature = "std"))]
    {
        false
    }
}

/// Match exactly a scalar integer iconst and a normal single-slot ABI arg.
/// In particular `get_constant` alone would also accept FP bit patterns.
fn constant(dfg: &DataFlowGraph, value: Value, abi: &ABIArg) -> Option<(OperandSize, u64)> {
    let ty = dfg.value_type(value);
    if !matches!(ty, types::I8 | types::I16 | types::I32 | types::I64) {
        return None;
    }
    let ABIArg::Slots { slots, purpose } = abi else {
        return None;
    };
    if *purpose != ArgumentPurpose::Normal || slots.len() != 1 || slots[0].get_type() != ty {
        return None;
    }
    let inst = dfg.value_def(value).inst()?;
    if dfg.inst_results(inst) != [value] {
        return None;
    }
    let InstructionData::UnaryImm {
        opcode: Opcode::Iconst,
        imm,
    } = dfg.insts[inst]
    else {
        return None;
    };
    let bits = imm.bits() as u64;
    let bits = if ty.bits() < 64 {
        bits & ((1u64 << ty.bits()) - 1)
    } else {
        bits
    };
    let size = if ty == types::I64 {
        OperandSize::Size64
    } else {
        OperandSize::Size32
    };
    Some((size, bits))
}

pub(super) fn lower_args(
    ctx: &mut Lower<Inst>,
    sig: Sig,
    patchable: bool,
    (list, offset): ValueSlice,
    enabled: bool,
) -> ValueRegsVec {
    let count = list.len(&ctx.dfg().value_lists) - offset;
    assert_eq!(count, ctx.sigs().num_args(sig));
    let mut copies = 0;
    (0..count)
        .map(|index| {
            let value = list.get(offset + index, &ctx.dfg().value_lists).unwrap();
            let candidate = if enabled
                && !patchable
                && copies < MAX_PER_CALL
                && ctx.x64_call_constant_remats < MAX_PER_FUNCTION
            {
                constant(ctx.dfg(), value, &ctx.sigs().args(sig)[index])
            } else {
                None
            };
            if let Some((size, bits)) = candidate {
                // This is the current call's machine sequence, after CLIF
                // optimization. Do not also mark the original iconst used.
                let temporary = ctx
                    .alloc_tmp(ctx.dfg().value_type(value))
                    .only_reg()
                    .unwrap();
                ctx.emit(Inst::imm(size, bits, temporary));
                copies += 1;
                ctx.x64_call_constant_remats += 1;
                ValueRegs::one(temporary.to_reg())
            } else {
                ctx.put_value_in_regs(value)
            }
        })
        .collect()
}

#[cfg(test)]
mod tests;
