//! Fold a single-use register AND into its equality-to-zero branch test.
//!
//! This runs after graph observations and width canonicalization are lowered.
//! Loads, calls and their order remain intact; only dead pure AND/copy results
//! disappear. Narrowing is legal because the adjacent branch observes only ZF.
use super::private_slots::{clean_dead_values, facts};
use super::*;

pub(super) fn run(stream: &mut Vec<LowerInstr>) -> usize {
    // The lowered protocol gives every flags consumer its own adjacent test.
    // Preserve streams with any other flags use, including a second branch
    // reading flags left by an earlier test.
    if stream.iter().enumerate().any(|(index, instruction)| {
        matches!(instruction, LowerInstr::Scalar(VirtualInstr::Jcc { .. }))
            && !index.checked_sub(1).is_some_and(|previous| {
                matches!(
                    stream[previous],
                    LowerInstr::Scalar(VirtualInstr::Test { .. })
                        | LowerInstr::TestLowByte { .. }
                        | LowerInstr::TestMasked { .. }
                )
            })
    }) {
        return 0;
    }
    let mut definitions = HashMap::<VirtualReg, Vec<usize>>::default();
    let mut uses = HashMap::<VirtualReg, usize>::default();
    for (index, instruction) in stream.iter().enumerate() {
        let instruction = facts(instruction);
        for reg in instruction.defs {
            definitions.entry(reg).or_default().push(index);
        }
        for reg in instruction.uses {
            *uses.entry(reg).or_default() += 1;
        }
    }
    let single = |value: VirtualReg| match definitions.get(&value).map(Vec::as_slice) {
        Some(&[index]) => Some(index),
        _ => None,
    };
    let mut replacements = Vec::new();
    for (index, pair) in stream.windows(2).enumerate() {
        let [
            LowerInstr::Scalar(VirtualInstr::Test {
                lhs: VirtualValue::Reg(tested),
                rhs: VirtualValue::Imm(0),
            }),
            LowerInstr::Scalar(VirtualInstr::Jcc {
                condition: Condition::Eq | Condition::NotEq,
                ..
            }),
        ] = pair
        else {
            continue;
        };
        if uses.get(tested) != Some(&1) {
            continue;
        }
        let Some(mut producer) = single(*tested) else {
            continue;
        };
        // Every bypassed copy must disappear, and the new source must have
        // one definition so a later assignment cannot change the tested bits.
        let mut budget = 32;
        while let LowerInstr::Scalar(VirtualInstr::Mov {
            src: VirtualValue::Reg(copied),
            ..
        }) = stream[producer]
        {
            if budget == 0 || uses.get(&copied) != Some(&1) {
                break;
            }
            let Some(next) = single(copied) else { break };
            producer = next;
            budget -= 1;
        }
        let LowerInstr::Scalar(VirtualInstr::And { lhs, rhs, .. }) = stream[producer] else {
            continue;
        };
        let (value, mask) = match (lhs, rhs) {
            (VirtualValue::Reg(value), VirtualValue::Imm(mask))
            | (VirtualValue::Imm(mask), VirtualValue::Reg(value)) => (value, mask),
            _ => continue,
        };
        if single(value).is_none() {
            continue;
        }
        let Some((mask, width)) = immediate(mask) else {
            continue;
        };
        replacements.push((
            index,
            *tested,
            LowerInstr::TestMasked { value, mask, width },
        ));
    }
    let count = replacements.len();
    if count == 0 {
        return 0;
    }
    let mut dead_seeds = Vec::with_capacity(count);
    for (index, tested, replacement) in replacements {
        stream[index] = replacement;
        dead_seeds.push(tested);
    }
    let mut removed = vec![false; stream.len()];
    clean_dead_values(stream, &mut removed, dead_seeds);
    let mut index = 0;
    stream.retain(|_| {
        let keep = !removed[index];
        index += 1;
        keep
    });
    count
}

fn immediate(mask: i64) -> Option<(u32, BitWidth)> {
    let bits = mask as u64;
    let width = if bits == 0 {
        return None;
    } else if bits <= u64::from(u8::MAX) {
        BitWidth::W8
    } else if bits <= u64::from(u32::MAX) {
        // TEST32 covers these bits without a 16-bit operand-size prefix.
        BitWidth::W32
    } else if i32::try_from(mask).is_ok() {
        // TEST64 sign-extends its imm32. Other 64-bit masks need a register
        // materialization and are left to the original AND sequence.
        BitWidth::W64
    } else {
        return None;
    };
    Some((bits as u32, width))
}

#[cfg(test)]
#[path = "masked_tests_tests.rs"]
mod tests;
