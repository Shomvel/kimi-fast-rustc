//! One constrained allocation/emission path for scalar and memory operations.
mod allocation;
mod checker;
mod constraints;
mod emit;
mod moves;
mod scan;

use super::*;
use crate::analysis::liveness::{self, Flow, InstructionLiveness};
use crate::machine::regalloc::{self, LiveInterval, RegisterAllocator};
use constraints::{Constraint, Instruction, Operation, Phase};
use moves::{Copy, Location};

pub(super) fn compile(
    graph: &FunctionGraph,
    stream: Vec<LowerInstr>,
    virtual_count: u32,
    stack_layouts: &[SizeAlign],
) -> Result<MachineFunction, LowerError> {
    let instructions = constraints::select(stream);
    let allocation = allocation::Allocation::new(&instructions, virtual_count)?;
    let mut emitter = emit::Emitter::new(&graph.symbol, &allocation, stack_layouts)?;
    let mut next_label = instructions
        .iter()
        .filter_map(|i| {
            if let Operation::Label(n) = i.operation {
                Some(n)
            } else {
                None
            }
        })
        .max()
        .unwrap_or(0)
        .checked_add(1)
        .ok_or_else(|| LowerError::Unsupported("label space exhausted".into()))?;
    let mut edges = Vec::new();
    let mut inverted_jump = None;
    for (position, instruction) in instructions.iter().enumerate() {
        let successors = &allocation.liveness.successors[position];
        if inverted_jump == Some(position) {
            continue;
        }
        match instruction.operation {
            Operation::Jump(_) => {
                emitter.edge(position, successors[0])?;
                emitter.instruction(instruction, position)?;
            }
            Operation::Branch(condition, target) => {
                let taken = successors[0];
                if allocation.edge(position, taken)?.is_empty() {
                    emitter.instruction(instruction, position)?;
                } else if let Some(next_target) =
                    copy_free_jump(&instructions, &allocation, position)?
                {
                    // The fallthrough only jumps on without copies. Branch
                    // there directly on the inverse condition and place the
                    // taken edge's copies inline, so no edge block is needed.
                    let mut branch = instruction.clone();
                    branch.operation =
                        Operation::Branch(super::branch::inverse(condition), next_target);
                    emitter.instruction(&branch, position)?;
                    emitter.edge(position, taken)?;
                    let mut jump = instruction.clone();
                    jump.operation = Operation::Jump(target);
                    emitter.instruction(&jump, position)?;
                    inverted_jump = Some(position + 1);
                    continue;
                } else {
                    let mut branch = instruction.clone();
                    branch.operation = Operation::Branch(condition, next_label);
                    emitter.instruction(&branch, position)?;
                    edges.push((next_label, position, taken, target));
                    next_label = next_label
                        .checked_add(1)
                        .ok_or_else(|| LowerError::Unsupported("label space exhausted".into()))?;
                }
                if let Some(&fallthrough) = successors.get(1) {
                    emitter.edge(position, fallthrough)?;
                }
            }
            Operation::Return => emitter.instruction(instruction, position)?,
            _ => {
                emitter.instruction(instruction, position)?;
                if let Some(&next) = successors.first() {
                    emitter.edge(position, next)?;
                }
            }
        }
    }
    for (label, source, target, destination) in edges {
        emitter.edge_block(label, source, target, destination)?;
    }
    let mut function = emitter.finish();
    crate::machine::x86::optimize_zero_idioms(&mut function.instructions);
    Ok(function)
}

/// The label of a `Jump` directly after the branch at `position` when neither
/// the fallthrough edge into it nor its own edge needs copies.
fn copy_free_jump(
    instructions: &[Instruction],
    allocation: &allocation::Allocation,
    position: usize,
) -> Result<Option<u32>, LowerError> {
    let next = position + 1;
    let Some(Operation::Jump(label)) = instructions.get(next).map(|i| &i.operation) else {
        return Ok(None);
    };
    let successors = &allocation.liveness.successors;
    if successors[position].get(1) != Some(&next)
        || !allocation.edge(position, next)?.is_empty()
        || !allocation.edge(next, successors[next][0])?.is_empty()
    {
        return Ok(None);
    }
    Ok(Some(*label))
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod intrinsic_tests;
