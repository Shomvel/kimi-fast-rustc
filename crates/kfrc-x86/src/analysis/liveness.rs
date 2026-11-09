//! Conservative live intervals over the selected instruction CFG.
use std::collections::{BTreeSet, HashMap, HashSet};

use crate::machine::regalloc::{LiveInterval, VirtualReg as AllocationReg};
use crate::machine::x86::{VirtualInstr, VirtualReg, VirtualValue};

pub(crate) fn defined(instruction: &VirtualInstr) -> Option<VirtualReg> {
    use VirtualInstr::*;
    match instruction {
        LoadArg { dst, .. }
        | Mov { dst, .. }
        | Neg { dst, .. }
        | Not { dst, .. }
        | Add { dst, .. }
        | Sub { dst, .. }
        | Mul { dst, .. }
        | And { dst, .. }
        | Or { dst, .. }
        | Xor { dst, .. }
        | Shl { dst, .. }
        | Shr { dst, .. }
        | LShr { dst, .. }
        | Div { dst, .. }
        | Rem { dst, .. }
        | UDiv { dst, .. }
        | URem { dst, .. }
        | Compare { dst, .. }
        | Load { dst, .. }
        | Phi { dst }
        | Call { dst, .. } => Some(*dst),
        Test { .. } | Nop | Store { .. } | Label(_) | Jmp(_) | Jcc { .. } | Return(_) => None,
    }
}

pub(crate) fn used(instruction: &VirtualInstr) -> Vec<VirtualReg> {
    use VirtualInstr::*;
    let mut registers = Vec::new();
    let mut add = |value: &VirtualValue| {
        if let VirtualValue::Reg(register) = value {
            registers.push(*register);
        }
    };
    match instruction {
        Mov { src, .. } | Neg { src, .. } | Not { src, .. } | Return(src) | Store { src, .. } => {
            add(src)
        }
        Add { lhs, rhs, .. }
        | Sub { lhs, rhs, .. }
        | Mul { lhs, rhs, .. }
        | And { lhs, rhs, .. }
        | Or { lhs, rhs, .. }
        | Xor { lhs, rhs, .. }
        | Shl { lhs, rhs, .. }
        | Shr { lhs, rhs, .. }
        | LShr { lhs, rhs, .. }
        | Div { lhs, rhs, .. }
        | Rem { lhs, rhs, .. }
        | UDiv { lhs, rhs, .. }
        | URem { lhs, rhs, .. }
        | Compare { lhs, rhs, .. }
        | Test { lhs, rhs } => {
            add(lhs);
            add(rhs);
        }
        Call { args, .. } => args.iter().for_each(&mut add),
        LoadArg { .. } | Load { .. } | Phi { .. } | Nop | Label(_) | Jmp(_) | Jcc { .. } => {}
    }
    registers
}

/// One conservative range per virtual register: the hull of every position
/// where it is defined, used, or live across a block boundary, from a
/// standard backward dataflow over the instruction CFG.
pub(crate) fn live_intervals(instructions: &[VirtualInstr]) -> Vec<LiveInterval> {
    let facts: Vec<_> = instructions
        .iter()
        .map(InstructionLiveness::scalar)
        .collect();
    intervals(&facts)
}

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

impl InstructionLiveness {
    pub(crate) fn scalar(instruction: &VirtualInstr) -> Self {
        Self {
            definitions: defined(instruction).into_iter().collect(),
            uses: used(instruction),
            flow: match instruction {
                VirtualInstr::Label(label) => Flow::Label(*label),
                VirtualInstr::Jmp(label) => Flow::Jump(*label),
                VirtualInstr::Jcc { target, .. } => Flow::Branch(*target),
                VirtualInstr::Return(_) => Flow::Return,
                _ => Flow::Next,
            },
        }
    }
}

pub(crate) fn intervals(instructions: &[InstructionLiveness]) -> Vec<LiveInterval> {
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
    let end_of = |block: usize| starts.get(block + 1).copied().unwrap_or(instructions.len());
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
        let end = end_of(block);
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
    // Upward-exposed uses and definitions per block.
    let mut uses = vec![HashSet::new(); block_count];
    let mut defs = vec![HashSet::new(); block_count];
    for block in 0..block_count {
        for instruction in &instructions[starts[block]..end_of(block)] {
            for &register in &instruction.uses {
                if !defs[block].contains(&register) {
                    uses[block].insert(register);
                }
            }
            for &register in &instruction.definitions {
                defs[block].insert(register);
            }
        }
    }
    let mut live_in: Vec<HashSet<VirtualReg>> = vec![HashSet::new(); block_count];
    let mut live_out: Vec<HashSet<VirtualReg>> = vec![HashSet::new(); block_count];
    let mut changed = true;
    while changed {
        changed = false;
        for block in (0..block_count).rev() {
            let mut out = HashSet::new();
            for successor in &successors[block] {
                out.extend(live_in[*successor].iter().copied());
            }
            let mut incoming = uses[block].clone();
            for register in &out {
                if !defs[block].contains(register) {
                    incoming.insert(*register);
                }
            }
            if incoming != live_in[block] || out != live_out[block] {
                live_in[block] = incoming;
                live_out[block] = out;
                changed = true;
            }
        }
    }
    let mut ranges: HashMap<u32, (u32, u32)> = HashMap::new();
    let mut extend = |register: VirtualReg, position: usize| {
        let position = position as u32;
        ranges
            .entry(register.0)
            .and_modify(|(start, end)| {
                *start = (*start).min(position);
                *end = (*end).max(position + 1);
            })
            .or_insert((position, position + 1));
    };
    for block in 0..block_count {
        let (start, end) = (starts[block], end_of(block));
        for register in &live_in[block] {
            extend(*register, start);
        }
        for register in &live_out[block] {
            extend(*register, end - 1);
        }
        for (index, instruction) in instructions[start..end].iter().enumerate() {
            for &register in &instruction.uses {
                extend(register, start + index);
            }
            for &register in &instruction.definitions {
                extend(register, start + index);
            }
        }
    }
    let mut intervals: Vec<LiveInterval> = ranges
        .into_iter()
        .map(|(register, (start, end))| LiveInterval::new(AllocationReg(register), start, end))
        .collect();
    intervals.sort_by_key(|interval| (interval.start, interval.virtual_reg.0));
    intervals
}
