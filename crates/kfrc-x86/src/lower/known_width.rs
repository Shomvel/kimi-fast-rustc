//! Remove width canonicalizations whose input already has the canonical form.
//!
//! Every Rust scalar lives in a 64-bit register as its zero- or sign-extended
//! image. A virtual register with one definition gets the number of low bits
//! outside which it is zero, from producers whose result range is exact:
//! constants, masks, `setcc`, zero-extending narrow loads and unsigned
//! extensions. An extension or low mask of such a value is a move; a mask
//! read only by an extension of no more bits than it keeps is bypassed.
//! A zero test of a byte extension or byte mask tests the low byte itself.
use super::private_slots::{Facts, clean_dead_values, facts};
use super::*;

#[derive(Default, Debug, PartialEq, Eq)]
pub(super) struct Stats {
    pub extensions_removed: usize,
    pub masks_removed: usize,
    pub masks_bypassed: usize,
    pub zero_tests_narrowed: usize,
}

struct Widths<'a> {
    stream: &'a [LowerInstr],
    definition: HashMap<VirtualReg, usize>,
    memo: HashMap<VirtualReg, Option<u32>>,
}

fn constant_width(value: i64) -> Option<u32> {
    (value >= 0).then(|| 64 - (value as u64).leading_zeros())
}

/// `Some(k)` for a low mask `2^k - 1` with `0 < k < 64`.
fn low_mask_width(mask: i64) -> Option<u32> {
    let mask = mask as u64;
    (mask != 0 && mask != u64::MAX && mask & (mask + 1) == 0).then(|| mask.trailing_ones())
}

impl Widths<'_> {
    fn value(&mut self, value: VirtualValue) -> Option<u32> {
        match value {
            VirtualValue::Imm(constant) => constant_width(constant),
            VirtualValue::Reg(reg) => self.reg(reg),
        }
    }

    /// The register's value is below `2^width` as an unsigned 64-bit integer.
    fn reg(&mut self, reg: VirtualReg) -> Option<u32> {
        if let Some(known) = self.memo.get(&reg) {
            return *known;
        }
        // A cycle through copies proves nothing.
        self.memo.insert(reg, None);
        let index = *self.definition.get(&reg)?;
        let width = match &self.stream[index] {
            LowerInstr::Scalar(instruction) => match *instruction {
                VirtualInstr::Mov { src, .. } => self.value(src),
                VirtualInstr::And { lhs, rhs, .. } => match (self.value(lhs), self.value(rhs)) {
                    (Some(left), Some(right)) => Some(left.min(right)),
                    (known, None) | (None, known) => known,
                },
                VirtualInstr::Or { lhs, rhs, .. } | VirtualInstr::Xor { lhs, rhs, .. } => {
                    Some(self.value(lhs)?.max(self.value(rhs)?))
                }
                VirtualInstr::LShr {
                    lhs,
                    rhs: VirtualValue::Imm(amount),
                    ..
                } if (0..64).contains(&amount) => {
                    Some(self.value(lhs).unwrap_or(64).saturating_sub(amount as u32))
                }
                VirtualInstr::UDiv { lhs, .. } => self.value(lhs),
                VirtualInstr::URem { lhs, rhs, .. } => match (self.value(lhs), self.value(rhs)) {
                    (Some(left), Some(right)) => Some(left.min(right)),
                    (known, None) | (None, known) => known,
                },
                VirtualInstr::Compare { .. } => Some(1),
                _ => None,
            },
            LowerInstr::Extend {
                value,
                width,
                signed: false,
                ..
            } => {
                let bits = width.bits();
                Some(self.reg(*value).map_or(bits, |known| known.min(bits)))
            }
            LowerInstr::Load { bytes, .. } if *bytes < 8 => Some(u32::from(*bytes) * 8),
            _ => None,
        };
        self.memo.insert(reg, width);
        width
    }
}

pub(super) fn run(stream: &mut Vec<LowerInstr>) -> Stats {
    let mut stats = Stats::default();
    let all_facts: Vec<_> = stream.iter().map(facts).collect();
    let mut definitions = HashMap::<VirtualReg, Vec<usize>>::new();
    let mut uses = HashMap::<VirtualReg, usize>::new();
    for (index, f) in all_facts.iter().enumerate() {
        for &reg in &f.defs {
            definitions.entry(reg).or_default().push(index);
        }
        for &reg in &f.uses {
            *uses.entry(reg).or_default() += 1;
        }
    }
    let definition: HashMap<_, _> = definitions
        .into_iter()
        .filter_map(|(reg, sites)| match sites.as_slice() {
            [index] => Some((reg, *index)),
            _ => None,
        })
        .collect();
    let mut widths = Widths {
        stream: stream.as_slice(),
        definition: definition.clone(),
        memo: HashMap::new(),
    };
    let mut replacements = Vec::new();
    for (index, instruction) in stream.iter().enumerate() {
        match *instruction {
            LowerInstr::Extend {
                dst,
                value,
                width,
                signed,
            } => {
                let known = widths.reg(value);
                let bits = width.bits();
                if known.is_some_and(|known| known < bits || (!signed && known == bits)) {
                    replacements.push((
                        index,
                        LowerInstr::Scalar(VirtualInstr::Mov {
                            dst,
                            src: VirtualValue::Reg(value),
                        }),
                    ));
                    stats.extensions_removed += 1;
                    continue;
                }
                // The extension reads only `bits` low bits; a single-use
                // mask that keeps all of them is not needed. The mask's
                // single-definition source holds the same value here.
                let Some(&mask_index) = definition.get(&value) else {
                    continue;
                };
                if let LowerInstr::Scalar(VirtualInstr::And {
                    lhs: VirtualValue::Reg(source),
                    rhs: VirtualValue::Imm(mask),
                    ..
                }) = stream[mask_index]
                    && uses.get(&value) == Some(&1)
                    && definition.contains_key(&source)
                    && low_mask_width(mask).is_some_and(|kept| kept >= bits)
                {
                    replacements.push((
                        index,
                        LowerInstr::Extend {
                            dst,
                            value: source,
                            width,
                            signed,
                        },
                    ));
                    stats.masks_bypassed += 1;
                }
            }
            LowerInstr::Scalar(VirtualInstr::And {
                dst,
                lhs,
                rhs: VirtualValue::Imm(mask),
            }) => {
                let src = match lhs {
                    VirtualValue::Imm(constant) => VirtualValue::Imm(constant & mask),
                    VirtualValue::Reg(_)
                        if low_mask_width(mask)
                            .zip(widths.value(lhs))
                            .is_some_and(|(kept, known)| known <= kept) =>
                    {
                        lhs
                    }
                    VirtualValue::Reg(_) => continue,
                };
                replacements.push((index, LowerInstr::Scalar(VirtualInstr::Mov { dst, src })));
                stats.masks_removed += 1;
            }
            _ => {}
        }
    }
    replace(stream, &all_facts, replacements);
    stats.zero_tests_narrowed = narrow_zero_tests(stream);
    stats
}

fn replace(
    stream: &mut Vec<LowerInstr>,
    all_facts: &[Facts],
    replacements: Vec<(usize, LowerInstr)>,
) {
    if replacements.is_empty() {
        return;
    }
    let mut dead_seeds = Vec::new();
    for (index, replacement) in replacements {
        dead_seeds.extend(all_facts[index].uses.iter().copied());
        stream[index] = replacement;
    }
    let mut removed = vec![false; stream.len()];
    clean_dead_values(stream, &mut removed, dead_seeds);
    let mut index = 0;
    stream.retain(|_| {
        let keep = !removed[index];
        index += 1;
        keep
    });
}

/// A zero test whose only operand is a byte extension or byte mask of a
/// single-definition register, read by an `Eq`/`NotEq` branch, tests that
/// register's low byte directly.
fn narrow_zero_tests(stream: &mut Vec<LowerInstr>) -> usize {
    let all_facts: Vec<_> = stream.iter().map(facts).collect();
    let mut definitions = HashMap::<VirtualReg, Vec<usize>>::new();
    let mut uses = HashMap::<VirtualReg, usize>::new();
    for (index, f) in all_facts.iter().enumerate() {
        for &reg in &f.defs {
            definitions.entry(reg).or_default().push(index);
        }
        for &reg in &f.uses {
            *uses.entry(reg).or_default() += 1;
        }
    }
    let single = |reg: VirtualReg| match definitions.get(&reg).map(Vec::as_slice) {
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
        let Some(mut definition) = single(*tested) else {
            continue;
        };
        // Single-use copies between the byte producer and the test carry
        // the same value and die with the test.
        while let LowerInstr::Scalar(VirtualInstr::Mov {
            src: VirtualValue::Reg(copied),
            ..
        }) = stream[definition]
        {
            let Some(producer) = single(copied).filter(|_| uses.get(&copied) == Some(&1)) else {
                break;
            };
            definition = producer;
        }
        let source = match stream[definition] {
            LowerInstr::Extend {
                value,
                width: BitWidth::W8,
                ..
            } => value,
            LowerInstr::Scalar(VirtualInstr::And {
                lhs: VirtualValue::Reg(value),
                rhs: VirtualValue::Imm(0xff),
                ..
            }) => value,
            _ => continue,
        };
        if single(source).is_none() {
            continue;
        }
        replacements.push((index, LowerInstr::TestLowByte { value: source }));
    }
    let count = replacements.len();
    replace(stream, &all_facts, replacements);
    count
}

#[cfg(test)]
#[path = "known_width_tests.rs"]
mod tests;
