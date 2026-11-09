//! Exact live ranges over the selected instruction CFG, including lifetime holes.
use std::collections::{BTreeSet, HashMap, HashSet};

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
pub(crate) fn defined_before(instructions: &[InstructionLiveness]) -> Vec<Vec<VirtualReg>> {
    let blocks = Blocks::new(instructions);
    let block_count = blocks.starts.len();
    let end_of = |block: usize| {
        blocks
            .starts
            .get(block + 1)
            .copied()
            .unwrap_or(instructions.len())
    };
    let mut block_definitions = vec![HashSet::new(); block_count];
    for (block, definitions) in block_definitions.iter_mut().enumerate() {
        for instruction in &instructions[blocks.starts[block]..end_of(block)] {
            definitions.extend(instruction.definitions.iter().copied());
        }
    }
    let mut defined_out: Vec<HashSet<VirtualReg>> = vec![HashSet::new(); block_count];
    let mut pending: Vec<_> = (0..block_count).rev().collect();
    let mut queued = vec![true; block_count];
    while let Some(block) = pending.pop() {
        queued[block] = false;
        let mut out = block_definitions[block].clone();
        for &predecessor in &blocks.predecessors[block] {
            out.extend(defined_out[predecessor].iter().copied());
        }
        if out.len() != defined_out[block].len() {
            defined_out[block] = out;
            for &successor in &blocks.successors[block] {
                if !queued[successor] {
                    queued[successor] = true;
                    pending.push(successor);
                }
            }
        }
    }
    let mut before = vec![Vec::new(); instructions.len()];
    for block in 0..block_count {
        let mut defined = HashSet::new();
        for &predecessor in &blocks.predecessors[block] {
            defined.extend(defined_out[predecessor].iter().copied());
        }
        for position in blocks.starts[block]..end_of(block) {
            before[position] = sorted(&defined);
            defined.extend(instructions[position].definitions.iter().copied());
        }
    }
    before
}

/// Live ranges restricted to values that hold a program value. Every use must
/// be defined on some path; callers rewrite undefined uses before analysis.
pub(crate) fn analyze(instructions: &[InstructionLiveness]) -> Liveness {
    let blocks = Blocks::new(instructions);
    let starts = &blocks.starts;
    let block_count = starts.len();
    let end_of = |block: usize| starts.get(block + 1).copied().unwrap_or(instructions.len());
    let successors = &blocks.successors;
    let predecessors = &blocks.predecessors;
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
    // Only a changed live-in can affect predecessors. Reverse block order is
    // the initial traversal; loops requeue their predecessors as needed.
    let mut pending: Vec<_> = (0..block_count).collect();
    let mut queued = vec![true; block_count];
    while let Some(block) = pending.pop() {
        queued[block] = false;
        let mut out = HashSet::new();
        for &successor in &successors[block] {
            out.extend(live_in[successor].iter().copied());
        }
        let mut incoming = uses[block].clone();
        incoming.extend(out.iter().filter(|v| !defs[block].contains(v)).copied());
        live_out[block] = out;
        if incoming != live_in[block] {
            live_in[block] = incoming;
            for &predecessor in &predecessors[block] {
                if !queued[predecessor] {
                    queued[predecessor] = true;
                    pending.push(predecessor);
                }
            }
        }
    }
    // Retain compact, sorted snapshots. Emission and checking query these
    // repeatedly; cloning a hash table and re-sorting it for every edge wastes
    // work proportional to its capacity rather than the number of live values.
    let mut before = vec![Vec::new(); instructions.len()];
    let mut after = before.clone();
    for block in 0..block_count {
        let mut live = live_out[block].clone();
        for position in (starts[block]..end_of(block)).rev() {
            after[position] = sorted(&live);
            for v in &instructions[position].definitions {
                live.remove(v);
            }
            live.extend(&instructions[position].uses);
            before[position] = sorted(&live);
        }
    }
    // A value live only along paths where it is undefined holds no program
    // value. Dropping those points removes garbage homes and edge copies.
    let defined = defined_before(instructions);
    for (position, instruction) in instructions.iter().enumerate() {
        let defined = &defined[position];
        let contains = |v: &VirtualReg| defined.binary_search_by_key(&v.0, |d| d.0).is_ok();
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

fn sorted(values: &HashSet<VirtualReg>) -> Vec<VirtualReg> {
    let mut values: Vec<_> = values.iter().copied().collect();
    values.sort_unstable_by_key(|v| v.0);
    values
}
