//! Def/use and control-flow facts for memory instruction selection.
use super::*;
use crate::analysis::liveness::{Flow, InstructionLiveness};

pub(super) fn facts(instruction: &LowerInstr) -> InstructionLiveness {
    use LowerInstr::*;
    let mut facts = InstructionLiveness::default();
    let mut values = Vec::new();
    match instruction {
        Scalar(instruction) => return InstructionLiveness::scalar(instruction),
        StackAddress { dst, .. } | SymbolAddress { dst, .. } | ThreadLocalAddress { dst, .. } => {
            facts.definitions.push(*dst);
        }
        LoadVector { address, .. } => values.push(*address),
        StoreVector { address, .. } => values.push(*address),
        Load { dst, address, .. } => {
            facts.definitions.push(*dst);
            values.push(*address);
        }
        Store { address, src, .. } => values.extend([*address, *src]),
        MulOverflow { dst, lhs, rhs, .. } => {
            facts.definitions.push(*dst);
            values.extend([*lhs, *rhs]);
        }
        CallPair {
            first,
            second,
            target,
            args,
        } => {
            facts.definitions.extend([*first, *second]);
            values.extend(args);
            if let CallDestination::Indirect(target) = target {
                values.push(*target);
            }
        }
        CallIndirect { dst, target, args } => {
            facts.definitions.push(*dst);
            values.push(*target);
            values.extend(args);
        }
        AbiCall {
            results,
            target,
            args,
            vector_args,
            vector_result,
        } => {
            facts.definitions.extend(results);
            values.extend(args.iter().map(|(_, value)| *value));
            values.extend(vector_args.iter().map(|(_, value)| *value));
            values.extend(vector_result);
            if let CallDestination::Indirect(target) = target {
                values.push(*target);
            }
        }
        ReturnPair { first, second } => {
            facts.flow = Flow::Return;
            values.extend([*first, *second]);
        }
    }
    facts.uses = values
        .into_iter()
        .filter_map(|value| match value {
            VirtualValue::Reg(register) => Some(register),
            VirtualValue::Imm(_) => None,
        })
        .collect();
    facts
}
