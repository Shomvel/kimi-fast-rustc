//! Exact live ranges over the selected instruction CFG, including lifetime holes.
use kfrc_body::fx_hash::HashMap;
use std::collections::BTreeSet;

use crate::machine::regalloc::{LiveInterval, VirtualReg as AllocationReg};
use crate::machine::x86::VirtualReg;

#[derive(Clone, Copy, Default)]
pub(crate) enum Flow {
    #[default]
    Next,
    Label(u32),
    Jump(u32),
    Branch(u32),
    Return,
}

#[derive(Default)]
pub(crate) struct InstructionLiveness {
    pub definitions: Vec<VirtualReg>,
    pub uses: Vec<VirtualReg>,
    pub flow: Flow,
}

pub(crate) struct Liveness {
    pub intervals: Vec<LiveInterval>,
    pub before: Vec<Vec<VirtualReg>>,
    pub after: Vec<Vec<VirtualReg>>,
    pub successors: Vec<Vec<usize>>,
}

struct Blocks {
    starts: Vec<usize>,
    successors: Vec<Vec<usize>>,
    predecessors: Vec<Vec<usize>>,
}
impl Blocks {
    fn new(instructions: &[InstructionLiveness]) -> Self {
        // Split into blocks at labels and after control transfers.
        let mut starts = BTreeSet::new();
        starts.insert(0usize);
        for (index, instruction) in instructions.iter().enumerate() {
            match instruction.flow {
                Flow::Label(_) => {
                    starts.insert(index);
                }
                Flow::Jump(_) | Flow::Branch(_) | Flow::Return => {
                    starts.insert(index + 1);
                }
                _ => {}
            }
        }
        starts.retain(|start| *start < instructions.len());
        let starts: Vec<usize> = starts.into_iter().collect();
        let block_count = starts.len();
        let block_at: HashMap<usize, usize> = starts
            .iter()
            .enumerate()
            .map(|(block, start)| (*start, block))
            .collect();
        let label_block: HashMap<u32, usize> = instructions
            .iter()
            .enumerate()
            .filter_map(|(index, instruction)| match instruction.flow {
                Flow::Label(label) => Some((label, block_at[&index])),
                _ => None,
            })
            .collect();
        let mut successors = vec![Vec::new(); block_count];
        for block in 0..block_count {
            let end = starts.get(block + 1).copied().unwrap_or(instructions.len());
            let mut falls_through = true;
            for instruction in &instructions[starts[block]..end] {
                match instruction.flow {
                    Flow::Jump(label) => {
                        successors[block].push(label_block[&label]);
                        falls_through = false;
                    }
                    Flow::Branch(target) => successors[block].push(label_block[&target]),
                    Flow::Return => falls_through = false,
                    _ => {}
                }
            }
            if falls_through && block + 1 < block_count {
                successors[block].push(block + 1);
            }
        }
        let mut predecessors = vec![Vec::new(); block_count];
        for (block, next) in successors.iter().enumerate() {
            for &target in next {
                predecessors[target].push(block);
            }
        }
        Self {
            starts,
            successors,
            predecessors,
        }
    }
}

/// Values with a definition on at least one path from the entry to each
/// instruction. A value outside this set at a point holds no program value
/// there: every path reaching it leaves the value undefined.
///
/// The set only grows inside a block, so it is the block-entry set plus the
/// value's definitions earlier in the same block. Block sets are bit sets over
/// the defined values, numbered densely in first-definition order.
pub(crate) struct DefinedBefore {
    block_of: Vec<u32>,
    block_start: Vec<usize>,
    /// Virtual register number -> dense value index, `u32::MAX` if never defined.
    dense: Vec<u32>,
    words: usize,
    /// `block_count * words` bits: values defined on some path into each block.
    entry: Vec<u64>,
    /// Ascending definition positions per dense value.
    definitions: Vec<Vec<usize>>,
}

impl DefinedBefore {
    pub(crate) fn new(instructions: &[InstructionLiveness]) -> Self {
        let blocks = Blocks::new(instructions);
        let block_count = blocks.starts.len();
        let end_of = |block: usize| {
            blocks
                .starts
                .get(block + 1)
                .copied()
                .unwrap_or(instructions.len())
        };
        let mut block_of = vec![0u32; instructions.len()];
        let mut dense: Vec<u32> = Vec::new();
        let mut definitions: Vec<Vec<usize>> = Vec::new();
        for block in 0..block_count {
            for position in blocks.starts[block]..end_of(block) {
                block_of[position] = block as u32;
                for register in &instructions[position].definitions {
                    let number = register.0 as usize;
                    if number >= dense.len() {
                        dense.resize(number + 1, u32::MAX);
                    }
                    if dense[number] == u32::MAX {
                        dense[number] = definitions.len() as u32;
                        definitions.push(Vec::new());
                    }
                    definitions[dense[number] as usize].push(position);
                }
            }
        }
        let words = definitions.len().div_ceil(64);
        let mut block_definitions = vec![0u64; block_count * words];
        for (value, positions) in definitions.iter().enumerate() {
            for &position in positions {
                block_definitions[block_of[position] as usize * words + value / 64] |=
                    1 << (value % 64);
            }
        }
        // Forward union to a fixed point; any queue order reaches the same sets.
        let mut defined_out = vec![0u64; block_count * words];
        let mut out = vec![0u64; words];
        let mut pending: Vec<_> = (0..block_count).rev().collect();
        let mut queued = vec![true; block_count];
        while let Some(block) = pending.pop() {
            queued[block] = false;
            out.copy_from_slice(&block_definitions[block * words..(block + 1) * words]);
            for &predecessor in &blocks.predecessors[block] {
                let incoming = &defined_out[predecessor * words..(predecessor + 1) * words];
                for (word, bits) in out.iter_mut().zip(incoming) {
                    *word |= bits;
                }
            }
            if out[..] != defined_out[block * words..(block + 1) * words] {
                defined_out[block * words..(block + 1) * words].copy_from_slice(&out);
                for &successor in &blocks.successors[block] {
                    if !queued[successor] {
                        queued[successor] = true;
                        pending.push(successor);
                    }
                }
            }
        }
        let mut entry = vec![0u64; block_count * words];
        for block in 0..block_count {
            for &predecessor in &blocks.predecessors[block] {
                for word in 0..words {
                    entry[block * words + word] |= defined_out[predecessor * words + word];
                }
            }
        }
        Self {
            block_of,
            block_start: blocks.starts,
            dense,
            words,
            entry,
            definitions,
        }
    }

    /// Whether `register` is defined on some path reaching `position`.
    pub(crate) fn contains(&self, position: usize, register: VirtualReg) -> bool {
        let Some(&value) = self.dense.get(register.0 as usize) else {
            return false;
        };
        if value == u32::MAX {
            return false;
        }
        let value = value as usize;
        let block = self.block_of[position] as usize;
        if self.entry[block * self.words + value / 64] & (1 << (value % 64)) != 0 {
            return true;
        }
        let positions = &self.definitions[value];
        let earlier = positions.partition_point(|&definition| definition < position);
        earlier > 0 && positions[earlier - 1] >= self.block_start[block]
    }
}

/// Live ranges restricted to values that hold a program value. Every use must
/// be defined on some path; callers rewrite undefined uses before analysis.
/// Sorted live-before and live-after sets of every instruction.
fn live_snapshots(
    instructions: &[InstructionLiveness],
    blocks: &Blocks,
) -> (Vec<Vec<VirtualReg>>, Vec<Vec<VirtualReg>>) {
    let starts = &blocks.starts;
    let block_count = starts.len();
    let end_of = |block: usize| starts.get(block + 1).copied().unwrap_or(instructions.len());
    let successors = &blocks.successors;
    let predecessors = &blocks.predecessors;
    // Block sets are bit sets over the registers that occur, numbered in
    // register order so a set reads out already sorted.
    let mut registers: Vec<VirtualReg> = instructions
        .iter()
        .flat_map(|instruction| instruction.definitions.iter().chain(&instruction.uses))
        .copied()
        .collect();
    registers.sort_unstable_by_key(|v| v.0);
    registers.dedup();
    let mut dense = vec![u32::MAX; registers.last().map_or(0, |v| v.0 as usize + 1)];
    for (index, register) in registers.iter().enumerate() {
        dense[register.0 as usize] = index as u32;
    }
    let words = registers.len().div_ceil(64);
    let bit = |register: VirtualReg| {
        let index = dense[register.0 as usize] as usize;
        (index / 64, 1u64 << (index % 64))
    };
    // Upward-exposed uses and definitions per block.
    let mut uses = vec![0u64; block_count * words];
    let mut defs = vec![0u64; block_count * words];
    for block in 0..block_count {
        let base = block * words;
        for instruction in &instructions[starts[block]..end_of(block)] {
            for &register in &instruction.uses {
                let (word, mask) = bit(register);
                if defs[base + word] & mask == 0 {
                    uses[base + word] |= mask;
                }
            }
            for &register in &instruction.definitions {
                let (word, mask) = bit(register);
                defs[base + word] |= mask;
            }
        }
    }
    let mut live_in = vec![0u64; block_count * words];
    let mut live_out = vec![0u64; block_count * words];
    // Only a changed live-in can affect predecessors. Reverse block order is
    // the initial traversal; loops requeue their predecessors as needed. The
    // least fixed point does not depend on the order.
    let mut pending: Vec<_> = (0..block_count).collect();
    let mut queued = vec![true; block_count];
    let mut out = vec![0u64; words];
    while let Some(block) = pending.pop() {
        queued[block] = false;
        out.fill(0);
        for &successor in &successors[block] {
            for (word, bits) in out.iter_mut().zip(&live_in[successor * words..]) {
                *word |= bits;
            }
        }
        let base = block * words;
        live_out[base..base + words].copy_from_slice(&out);
        let mut changed = false;
        for word in 0..words {
            let incoming = uses[base + word] | (out[word] & !defs[base + word]);
            if incoming != live_in[base + word] {
                live_in[base + word] = incoming;
                changed = true;
            }
        }
        if changed {
            for &predecessor in &predecessors[block] {
                if !queued[predecessor] {
                    queued[predecessor] = true;
                    pending.push(predecessor);
                }
            }
        }
    }
    // Retain compact, sorted snapshots. Emission and checking query these
    // repeatedly. One backward walk per block keeps the live set sorted and
    // updates it in place, so a snapshot is a copy, not a sort.
    let mut before = vec![Vec::new(); instructions.len()];
    let mut after = before.clone();
    for block in 0..block_count {
        let mut live: Vec<VirtualReg> = Vec::new();
        for (word, &bits) in live_out[block * words..(block + 1) * words]
            .iter()
            .enumerate()
        {
            let mut bits = bits;
            while bits != 0 {
                live.push(registers[word * 64 + bits.trailing_zeros() as usize]);
                bits &= bits - 1;
            }
        }
        for position in (starts[block]..end_of(block)).rev() {
            after[position] = live.clone();
            for v in &instructions[position].definitions {
                if let Ok(index) = live.binary_search_by_key(&v.0, |l| l.0) {
                    live.remove(index);
                }
            }
            for v in &instructions[position].uses {
                if let Err(index) = live.binary_search_by_key(&v.0, |l| l.0) {
                    live.insert(index, *v);
                }
            }
            before[position] = live.clone();
        }
    }
    (before, after)
}

pub(crate) fn analyze(instructions: &[InstructionLiveness]) -> Liveness {
    let blocks = Blocks::new(instructions);
    let (mut before, mut after) = live_snapshots(instructions, &blocks);
    // A value live only along paths where it is undefined holds no program
    // value. Dropping those points removes garbage homes and edge copies.
    let defined = DefinedBefore::new(instructions);
    for (position, instruction) in instructions.iter().enumerate() {
        let contains = |v: &VirtualReg| defined.contains(position, *v);
        before[position].retain(|v| contains(v) || instruction.uses.contains(v));
        after[position].retain(|v| contains(v) || instruction.definitions.contains(v));
    }
    // A value needs a home at its uses, at definitions (including dead ones),
    // and while live through an instruction. Live-before plus definitions
    // equals that union because live-after - definitions is live-before.
    // Build separate half-open ranges, not a hull across mutually exclusive
    // blocks. Real CFG edges reconcile homes; textual gaps need no reload.
    let count = instructions
        .iter()
        .flat_map(|i| i.definitions.iter().chain(&i.uses))
        .map(|v| v.0 as usize + 1)
        .max()
        .unwrap_or(0);
    // A value dead before a redefinition (a Phi copied again on a back edge)
    // starts a new range there: its old register may then pass to the value
    // computed from it in between.
    let mut ranges = vec![Vec::<(u32, u32)>::new(); count];
    for (position, instruction) in instructions.iter().enumerate() {
        let live_in = &before[position];
        for &v in live_in.iter().chain(&instruction.definitions) {
            let continues = |end: u32| {
                end > position as u32 || live_in.binary_search_by_key(&v.0, |l| l.0).is_ok()
            };
            let position = position as u32;
            let parts = &mut ranges[v.0 as usize];
            if let Some((_, end)) = parts.last_mut()
                && *end >= position
                && continues(*end)
            {
                *end = position + 1;
            } else {
                parts.push((position, position + 1));
            }
        }
    }
    let mut intervals = ranges
        .into_iter()
        .enumerate()
        .flat_map(|(v, parts)| {
            parts
                .into_iter()
                .map(move |(start, end)| LiveInterval::new(AllocationReg(v as u32), start, end))
        })
        .collect::<Vec<_>>();
    intervals.sort_by_key(|interval| (interval.start, interval.virtual_reg.0));
    let labels: HashMap<_, _> = instructions
        .iter()
        .enumerate()
        .filter_map(|(p, i)| {
            if let Flow::Label(label) = i.flow {
                Some((label, p))
            } else {
                None
            }
        })
        .collect();
    let successors = instructions
        .iter()
        .enumerate()
        .map(|(p, i)| {
            let mut next = Vec::new();
            match i.flow {
                Flow::Jump(label) => next.push(labels[&label]),
                Flow::Return => {}
                Flow::Branch(label) => {
                    next.push(labels[&label]);
                    if p + 1 < instructions.len() {
                        next.push(p + 1);
                    }
                }
                _ => {
                    if p + 1 < instructions.len() {
                        next.push(p + 1);
                    }
                }
            }
            next
        })
        .collect();
    Liveness {
        intervals,
        before,
        after,
        successors,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kfrc_body::fx_hash::HashSet;

    /// The per-instruction sets DefinedBefore replaces, computed directly.
    fn reference(instructions: &[InstructionLiveness]) -> Vec<HashSet<VirtualReg>> {
        let blocks = Blocks::new(instructions);
        let count = blocks.starts.len();
        let end_of = |block: usize| {
            blocks
                .starts
                .get(block + 1)
                .copied()
                .unwrap_or(instructions.len())
        };
        let mut out: Vec<HashSet<VirtualReg>> = vec![HashSet::default(); count];
        loop {
            let mut changed = false;
            for block in 0..count {
                let mut next: HashSet<VirtualReg> = HashSet::default();
                for &predecessor in &blocks.predecessors[block] {
                    next.extend(out[predecessor].iter().copied());
                }
                for instruction in &instructions[blocks.starts[block]..end_of(block)] {
                    next.extend(instruction.definitions.iter().copied());
                }
                if next != out[block] {
                    out[block] = next;
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
        let mut before = Vec::new();
        for block in 0..count {
            let mut defined: HashSet<VirtualReg> = HashSet::default();
            for &predecessor in &blocks.predecessors[block] {
                defined.extend(out[predecessor].iter().copied());
            }
            for instruction in &instructions[blocks.starts[block]..end_of(block)] {
                before.push(defined.clone());
                defined.extend(instruction.definitions.iter().copied());
            }
        }
        before
    }

    /// Generated instruction streams: random definitions, uses and flow,
    /// with every jump target label present once.
    fn generated_streams(count: usize) -> Vec<(Vec<InstructionLiveness>, u32)> {
        let mut state = 0x9e37_79b9_7f4a_7c15u64;
        let mut next = |bound: u64| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state % bound
        };
        (0..count)
            .map(|_| {
                let length = 1 + next(80) as usize;
                let labels = 1 + next(10) as u32;
                let registers = 1 + next(150) as u32;
                let mut instructions: Vec<_> = (0..length)
                    .map(|_| InstructionLiveness {
                        definitions: (0..next(3))
                            .map(|_| VirtualReg(next(registers as u64) as u32))
                            .collect(),
                        uses: (0..next(4))
                            .map(|_| VirtualReg(next(registers as u64) as u32))
                            .collect(),
                        flow: match next(10) {
                            0 => Flow::Jump(next(labels as u64) as u32),
                            1 => Flow::Branch(next(labels as u64) as u32),
                            2 => Flow::Return,
                            _ => Flow::Next,
                        },
                    })
                    .chain((0..labels).map(|label| InstructionLiveness {
                        flow: Flow::Label(label),
                        ..InstructionLiveness::default()
                    }))
                    .collect();
                for index in (1..instructions.len()).rev() {
                    let other = next(index as u64 + 1) as usize;
                    instructions.swap(index, other);
                }
                (instructions, registers)
            })
            .collect()
    }

    #[test]
    fn live_snapshots_match_hash_set_dataflow_on_generated_flow() {
        for (instructions, _) in generated_streams(300) {
            let blocks = Blocks::new(&instructions);
            let count = blocks.starts.len();
            let end_of = |block: usize| {
                blocks
                    .starts
                    .get(block + 1)
                    .copied()
                    .unwrap_or(instructions.len())
            };
            // Reference: iterate hash-set equations to the least fixed point.
            let mut live_in: Vec<HashSet<VirtualReg>> = vec![HashSet::default(); count];
            loop {
                let mut changed = false;
                for block in (0..count).rev() {
                    let mut live: HashSet<VirtualReg> = HashSet::default();
                    for &successor in &blocks.successors[block] {
                        live.extend(live_in[successor].iter().copied());
                    }
                    for instruction in instructions[blocks.starts[block]..end_of(block)]
                        .iter()
                        .rev()
                    {
                        for v in &instruction.definitions {
                            live.remove(v);
                        }
                        live.extend(instruction.uses.iter().copied());
                    }
                    if live != live_in[block] {
                        live_in[block] = live;
                        changed = true;
                    }
                }
                if !changed {
                    break;
                }
            }
            let (before, after) = live_snapshots(&instructions, &blocks);
            for block in 0..count {
                let mut live: HashSet<VirtualReg> = HashSet::default();
                for &successor in &blocks.successors[block] {
                    live.extend(live_in[successor].iter().copied());
                }
                for position in (blocks.starts[block]..end_of(block)).rev() {
                    let mut expected: Vec<_> = live.iter().copied().collect();
                    expected.sort_unstable_by_key(|v| v.0);
                    assert_eq!(after[position], expected, "after {position}");
                    for v in &instructions[position].definitions {
                        live.remove(v);
                    }
                    live.extend(instructions[position].uses.iter().copied());
                    let mut expected: Vec<_> = live.iter().copied().collect();
                    expected.sort_unstable_by_key(|v| v.0);
                    assert_eq!(before[position], expected, "before {position}");
                }
            }
        }
    }

    #[test]
    fn defined_before_matches_per_instruction_sets_on_generated_flow() {
        let mut state = 0x2545_f491_4f6c_dd1du64;
        let mut next = |bound: u64| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state % bound
        };
        for _ in 0..200 {
            let length = 1 + next(60) as usize;
            let labels = 1 + next(8) as u32;
            let registers = 1 + next(140) as u32;
            let instructions: Vec<_> = (0..length)
                .map(|_| InstructionLiveness {
                    definitions: (0..next(3))
                        .map(|_| VirtualReg(next(registers as u64) as u32))
                        .collect(),
                    uses: Vec::new(),
                    flow: match next(10) {
                        0 => Flow::Jump(next(labels as u64) as u32),
                        1 => Flow::Branch(next(labels as u64) as u32),
                        2 => Flow::Return,
                        _ => Flow::Next,
                    },
                })
                .chain((0..labels).map(|label| InstructionLiveness {
                    flow: Flow::Label(label),
                    ..InstructionLiveness::default()
                }))
                .collect();
            let mut instructions = instructions;
            // Spread the labels through the stream; every target label exists once.
            for index in (1..instructions.len()).rev() {
                let other = next(index as u64 + 1) as usize;
                instructions.swap(index, other);
            }
            let expected = reference(&instructions);
            let defined = DefinedBefore::new(&instructions);
            for (position, set) in expected.iter().enumerate() {
                for register in 0..registers + 2 {
                    let register = VirtualReg(register);
                    assert_eq!(
                        defined.contains(position, register),
                        set.contains(&register),
                        "position {position} register {register:?}"
                    );
                }
            }
        }
    }
}
