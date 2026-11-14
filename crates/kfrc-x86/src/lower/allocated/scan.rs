//! Split linear scan. Earlier fragments are immutable when pressure evicts a value.
use super::*;
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug)]
pub(super) struct Segment {
    pub start: usize,
    pub end: usize,
    pub location: Location,
}
#[derive(Clone, Copy)]
struct Pending {
    value: VirtualReg,
    start: usize,
    end: usize,
}

#[derive(Clone, Copy)]
struct Pin {
    position: usize,
    kind: PinKind,
}
#[derive(Clone, Copy)]
enum PinKind {
    Clobber { result: Option<VirtualReg> },
    Input(VirtualValue),
}

pub(super) fn scan(
    instructions: &[Instruction],
    liveness: &liveness::Liveness,
    count: u32,
) -> Result<Scan, LowerError> {
    let mut pending = BTreeMap::new();
    let mut segments = vec![Vec::<Segment>::new(); count as usize];
    let mut uses = vec![Vec::new(); count as usize];
    let mut pins = vec![Vec::new(); Reg::ALLOCATABLE.len()];
    let mut hints = HashMap::default();
    // Registers a value must occupy at a later use or at its definition.
    // Allocating there directly saves the copy the constraint would need.
    let mut fixed_uses = vec![Vec::new(); count as usize];
    let mut definition_registers = HashMap::default();
    for (position, instruction) in instructions.iter().enumerate() {
        for input in &instruction.inputs {
            if let (VirtualValue::Reg(v), Constraint::Fixed(reg)) = (input.value, input.constraint)
            {
                fixed_uses[v.0 as usize].push((position, reg));
            }
        }
        for output in &instruction.outputs {
            if let Constraint::Fixed(reg) = output.constraint {
                definition_registers.insert((position, output.value), reg);
            }
        }
        if let Operation::Entry(arguments) = &instruction.operation {
            for &(dst, index) in arguments {
                definition_registers.insert((position, dst), Reg::ARGUMENTS[index]);
            }
        }
        for v in instruction.facts().uses {
            uses[v.0 as usize].push(position);
        }
        for &r in &instruction.clobbers {
            let result = instruction.outputs.iter().find_map(|output| {
                matches!(output.constraint, Constraint::Fixed(reg) if reg == r)
                    .then_some(output.value)
            });
            pins[reg_index(r)].push(Pin {
                position,
                kind: PinKind::Clobber { result },
            });
        }
        for input in &instruction.inputs {
            if let Constraint::Fixed(reg) = input.constraint {
                pins[reg_index(reg)].push(Pin {
                    position,
                    kind: PinKind::Input(input.value),
                });
            }
        }
        for output in &instruction.outputs {
            // A three-address add keeps the two-address preference: sharing
            // a dying source's register keeps the shorter in-place form.
            let source = match (output.constraint, &instruction.operation) {
                (Constraint::Reuse(index), _) => Some(index),
                (Constraint::Any, Operation::BinaryImm(..)) => Some(0),
                _ => None,
            };
            if let Some(index) = source
                && let VirtualValue::Reg(source) = instruction.inputs[index].value
            {
                hints.insert((position, output.value), source);
            }
        }
        if let Operation::Parallel(copies) = &instruction.operation {
            for &(dst, src) in copies {
                if let VirtualValue::Reg(src) = src {
                    hints.insert((position, dst), src);
                }
            }
        }
    }
    for range in &liveness.intervals {
        let value = VirtualReg(range.virtual_reg.0);
        pending.insert(
            (range.start.0 as usize, value.0),
            Pending {
                value,
                start: range.start.0 as usize,
                end: range.end.0 as usize,
            },
        );
    }
    let mut predecessors = vec![Vec::new(); instructions.len()];
    for (source, targets) in liveness.successors.iter().enumerate() {
        for &target in targets {
            predecessors[target].push(source);
        }
    }
    let back_edges = back_edges(instructions, liveness);
    // Phi copies feeding a back edge: (position, destination, source).
    let mut loop_copies = vec![Vec::new(); count as usize];
    for &(jump, _) in &back_edges {
        if let Some(Operation::Parallel(copies)) =
            jump.checked_sub(1).map(|p| &instructions[p].operation)
        {
            for &(dst, src) in copies {
                if let VirtualValue::Reg(src) = src {
                    loop_copies[src.0 as usize].push((jump - 1, dst));
                }
            }
        }
    }
    let mut active: Vec<(VirtualReg, usize, Reg)> = Vec::new();
    let mut coalesced: Vec<(usize, VirtualReg)> = Vec::new();
    while let Some((_, current)) = pending.pop_first() {
        let start = current.start;
        active.retain(|&(v, index, _)| segments[v.0 as usize][index].end > start);
        let next_use = |v: VirtualReg, from: usize, end: usize| {
            let points = &uses[v.0 as usize];
            points
                .get(points.partition_point(|&p| p < from))
                .copied()
                .filter(|&p| p < end)
                .unwrap_or(usize::MAX)
        };
        let mut free_until = [usize::MAX; 14];
        let next_pin = |reg: Reg, from: usize| {
            next_blocking_pin(
                &pins[reg_index(reg)],
                current.value,
                from,
                current.end,
                start,
                instructions,
                liveness,
            )
        };
        for (index, &reg) in Reg::ALLOCATABLE.iter().enumerate() {
            free_until[index] = next_pin(reg, start);
        }
        for &(_, _, reg) in &active {
            free_until[reg_index(reg)] = start;
        }
        let hinted = hints.get(&(start, current.value)).copied();
        if coalesced.first().is_some_and(|&(p, _)| p != start) {
            coalesced.clear();
        }
        // An instruction reads its early inputs before writing any output,
        // so a value defined here may take over the register of an input that
        // dies here. Later values starting here still see that register held.
        let mut shared = Vec::new();
        let facts = instructions[start].facts();
        // A Phi destination can also be an input of the same parallel copy
        // (including v <- v on a loop back edge). Its old value must already
        // be in its home before this instruction. Sharing a dying input's
        // register would make the incoming edge copy overwrite that input
        // before the parallel copy reads it.
        if facts.definitions.contains(&current.value) && !facts.uses.contains(&current.value) {
            for &(v, part, reg) in &active {
                if segments[v.0 as usize][part].end == start + 1
                    && active.iter().filter(|&&(_, _, r)| r == reg).count() == 1
                    && dies_at_early_read(v, start, instructions, liveness)
                {
                    free_until[reg_index(reg)] = next_pin(reg, start);
                    shared.push(reg);
                }
            }
        }
        // At a block entry, the homes the value already has at the ends of
        // allocated (earlier) predecessors decide its entry home: staying in
        // the stack until the next use when every such predecessor holds it
        // there, else preferring a predecessor's register. Either choice
        // saves the copy each matching edge would otherwise need.
        let mut entry_register = None;
        if matches!(instructions[start].operation, Operation::Label(_)) {
            let homes: Vec<_> = predecessors[start]
                .iter()
                .filter(|&&p| p < start)
                .filter_map(|&p| fragment_location(&segments[current.value.0 as usize], p))
                .collect();
            if !homes.is_empty()
                && homes
                    .iter()
                    .all(|location| matches!(location, Location::Stack(_)))
            {
                let end = next_use(current.value, start, current.end).min(current.end);
                segments[current.value.0 as usize].push(Segment {
                    start,
                    end,
                    location: Location::Stack(StackSlot(0)),
                });
                if end < current.end {
                    pending.insert(
                        (end, current.value.0),
                        Pending {
                            start: end,
                            ..current
                        },
                    );
                }
                continue;
            }
            entry_register = homes.iter().find_map(|location| location.register());
        }
        // A loop-carried value should reach the back edge in the register it
        // already has at the loop header, and a Phi source in the register
        // of the Phi it feeds there, so the back edge needs no copies.
        let first_back_edge = back_edges.partition_point(|&(jump, _)| jump < start);
        let loop_register = back_edges[first_back_edge..]
            .iter()
            .take_while(|&&(jump, _)| jump < current.end)
            .find_map(|&(_, header)| {
                fragment_location(&segments[current.value.0 as usize], header)?.register()
            })
            .or_else(|| {
                loop_copies[current.value.0 as usize]
                    .iter()
                    .filter(|&&(copy, _)| start <= copy && copy < current.end)
                    .find_map(|&(copy, dst)| {
                        let header = liveness.successors[copy + 1][0];
                        fragment_location(&segments[dst.0 as usize], header)?.register()
                    })
            });
        let preferred = loop_register
            .or_else(|| {
                hinted
                    .and_then(|source| segments[source.0 as usize].last())
                    .and_then(|s| {
                        if s.end <= start + 1 {
                            s.location.register()
                        } else {
                            None
                        }
                    })
            })
            .or_else(|| definition_registers.get(&(start, current.value)).copied())
            .or(entry_register)
            .or_else(|| {
                let points = &fixed_uses[current.value.0 as usize];
                points
                    .get(points.partition_point(|&(p, _)| p < start))
                    .filter(|&&(p, _)| p < current.end)
                    .map(|&(_, reg)| reg)
            })
            // A lifetime hole permits reuse by other values; retaining the
            // old register as a soft preference avoids needless edge cycles
            // when it is free again on re-entry.
            .or_else(|| {
                segments[current.value.0 as usize]
                    .last()
                    .and_then(|s| s.location.register())
            });
        let best = Reg::ALLOCATABLE
            .iter()
            .copied()
            .max_by_key(|&reg| {
                let until = free_until[reg_index(reg)];
                (
                    until.min(current.end),
                    Some(reg) == preferred,
                    std::cmp::Reverse(reg_index(reg)),
                )
            })
            .unwrap();
        let available = free_until[reg_index(best)];
        if available > start {
            if shared.contains(&best) {
                // The register still holds the dying input at start, so the
                // new value must not be evicted there.
                coalesced.push((start, current.value));
            }
            let end = current.end.min(available);
            let index = segments[current.value.0 as usize].len();
            segments[current.value.0 as usize].push(Segment {
                start,
                end,
                location: Location::Reg(best),
            });
            active.push((current.value, index, best));
            if end < current.end {
                pending.insert(
                    (end, current.value.0),
                    Pending {
                        start: end,
                        ..current
                    },
                );
            }
            continue;
        }
        // Furthest next use loses the register. Only the suffix spills: never
        // rewrite a prefix or reuse a slot that was occupied in that prefix.
        let victim = active
            .iter()
            .enumerate()
            .filter(|(_, (v, _, r))| {
                next_pin(*r, start) != start && !coalesced.contains(&(start, *v))
            })
            .max_by_key(|(_, (v, index, _))| {
                (next_use(*v, start, segments[v.0 as usize][*index].end), v.0)
            })
            .map(|(i, _)| i);
        if let Some(victim) = victim
            && next_use(
                active[victim].0,
                start,
                segments[active[victim].0.0 as usize][active[victim].1].end,
            ) > next_use(current.value, start, current.end)
        {
            let (value, index, reg) = active.remove(victim);
            let end = segments[value.0 as usize][index].end;
            segments[value.0 as usize][index].end = start;
            let resume = next_use(value, start + 1, end).min(end);
            segments[value.0 as usize].push(Segment {
                start,
                end: resume,
                location: Location::Stack(StackSlot(0)),
            });
            if resume < end {
                pending.insert(
                    (resume, value.0),
                    Pending {
                        value,
                        start: resume,
                        end,
                    },
                );
            }
            let available = next_pin(reg, start + 1).min(current.end);
            let index = segments[current.value.0 as usize].len();
            segments[current.value.0 as usize].push(Segment {
                start,
                end: available,
                location: Location::Reg(reg),
            });
            active.push((current.value, index, reg));
            if available < current.end {
                pending.insert(
                    (available, current.value.0),
                    Pending {
                        start: available,
                        ..current
                    },
                );
            }
        } else {
            // A spilled value needs no register between uses. Requeue at the
            // next real use, not at every intervening instruction.
            let end = next_use(current.value, start + 1, current.end).min(current.end);
            segments[current.value.0 as usize].push(Segment {
                start,
                end,
                location: Location::Stack(StackSlot(0)),
            });
            if end < current.end {
                pending.insert(
                    (end, current.value.0),
                    Pending {
                        start: end,
                        ..current
                    },
                );
            }
        }
    }
    for parts in segments.iter_mut() {
        parts.retain(|s| s.start < s.end);
    }
    let memory = store_at_definition(instructions, liveness, &segments);
    // Color only actual spilled fragments. Their hull remains conservative
    // across loops; nonoverlapping values can reuse a slot. A value stored at
    // its definitions owns its slot over the hull of all its fragments: every
    // position where it is live relies on the slot, including register
    // fragments that reach a stack fragment again through a loop.
    let spills: Vec<_> = segments
        .iter()
        .enumerate()
        .filter_map(|(v, parts)| {
            let spilled: Vec<_> = parts
                .iter()
                .filter(|s| matches!(s.location, Location::Stack(_)))
                .collect();
            let (first, last) = if memory[v] {
                (parts.first()?, parts.last()?)
            } else {
                (*spilled.first()?, *spilled.last()?)
            };
            Some(LiveInterval::new(
                regalloc::VirtualReg(v as u32),
                first.start as u32,
                last.end as u32,
            ))
        })
        .collect();
    let slots = RegisterAllocator::new(0)
        .allocate(&spills)
        .map_err(|e| LowerError::Backend(e.into()))?;
    let mut memory_slots = vec![None; segments.len()];
    for (v, parts) in segments.iter_mut().enumerate() {
        let slot = slots
            .allocation(regalloc::VirtualReg(v as u32))
            .and_then(|l| l.spill_slot());
        for part in parts.iter_mut() {
            if matches!(part.location, Location::Stack(_)) {
                part.location = Location::Stack(StackSlot(slot.unwrap().0));
            }
        }
        let mut merged = Vec::<Segment>::new();
        for &part in parts.iter() {
            if let Some(last) = merged.last_mut()
                && last.end == part.start
                && last.location == part.location
            {
                last.end = part.end;
            } else {
                merged.push(part);
            }
        }
        *parts = merged;
        if memory[v] {
            memory_slots[v] = slot.map(|slot| StackSlot(slot.0));
        }
    }
    Ok(Scan {
        segments,
        spill_slots: slots.spill_slot_count,
        memory_slots,
    })
}

pub(super) struct Scan {
    pub segments: Vec<Vec<Segment>>,
    pub spill_slots: u32,
    /// The slot of each value stored once after every definition. That slot
    /// holds the value wherever it is live, so entering a stack fragment
    /// needs no store.
    pub memory_slots: Vec<Option<StackSlot>>,
}

/// Chooses values whose spill stores move to their definitions (C1's
/// storeAtDefinition). A value qualifies when it has fewer definitions with a
/// register home than CFG edges entering a stack fragment from a register.
fn store_at_definition(
    instructions: &[Instruction],
    liveness: &liveness::Liveness,
    segments: &[Vec<Segment>],
) -> Vec<bool> {
    let home = |v: usize, position: usize| {
        let parts = &segments[v];
        let index = parts.partition_point(|s| s.end <= position);
        parts
            .get(index)
            .filter(|s| s.start <= position)
            .map(|s| s.location)
    };
    let mut predecessors = vec![Vec::new(); instructions.len()];
    for (source, targets) in liveness.successors.iter().enumerate() {
        for &target in targets {
            predecessors[target].push(source);
        }
    }
    let mut definitions = vec![Vec::new(); segments.len()];
    for (position, instruction) in instructions.iter().enumerate() {
        for v in instruction.facts().definitions {
            if matches!(home(v.0 as usize, position), Some(Location::Reg(_))) {
                definitions[v.0 as usize].push(position);
            }
        }
    }
    let loops = back_edges(instructions, liveness);
    // A definition store inside a loop runs every iteration; keep it only
    // when the stores it replaces are in that loop as well.
    let hoists_into_loop = |definition: usize, store: usize| {
        loops.iter().any(|&(jump, header)| {
            (header..=jump).contains(&definition) && !(header..=jump).contains(&store)
        })
    };
    segments
        .iter()
        .enumerate()
        .map(|(v, parts)| {
            let mut stores = Vec::new();
            for part in parts {
                if !matches!(part.location, Location::Stack(_)) {
                    continue;
                }
                for &source in &predecessors[part.start] {
                    let carried = liveness.after[source]
                        .binary_search_by_key(&(v as u32), |value| value.0)
                        .is_ok();
                    if carried && matches!(home(v, source), Some(Location::Reg(_))) {
                        stores.push(source);
                    }
                }
            }
            stores.len() > definitions[v].len()
                && !definitions[v].iter().any(|&definition| {
                    stores
                        .iter()
                        .any(|&store| hoists_into_loop(definition, store))
                })
        })
        .collect()
}
/// Jumps to an earlier position, as (jump position, target position), in
/// increasing jump order. Positions between a target and its jump form the
/// textual extent of a loop.
fn back_edges(instructions: &[Instruction], liveness: &liveness::Liveness) -> Vec<(usize, usize)> {
    instructions
        .iter()
        .enumerate()
        .filter(|(_, instruction)| matches!(instruction.operation, Operation::Jump(_)))
        .filter_map(|(jump, _)| {
            let target = *liveness.successors[jump].first()?;
            (target <= jump).then_some((jump, target))
        })
        .collect()
}

/// The location of a value at `position` among its fragments so far.
fn fragment_location(parts: &[Segment], position: usize) -> Option<Location> {
    let index = parts.partition_point(|s| s.end <= position);
    parts
        .get(index)
        .filter(|s| s.start <= position)
        .map(|s| s.location)
}
fn reg_index(r: Reg) -> usize {
    Reg::ALLOCATABLE.iter().position(|&reg| reg == r).unwrap()
}

// This is a conservative instruction-granularity constraint, not LLVM's full
// SlotIndexes model. Fixed inputs only exclude other live-through values; the
// required value and values defined after the input read remain eligible.
fn next_blocking_pin(
    points: &[Pin],
    value: VirtualReg,
    from: usize,
    end: usize,
    fragment_start: usize,
    instructions: &[Instruction],
    liveness: &liveness::Liveness,
) -> usize {
    let first = points.partition_point(|pin| pin.position < from);
    // A later pin cannot affect this fragment. Bound the scan so short-lived
    // values do not walk every subsequent fixed-input instruction in a block.
    let last = first + points[first..].partition_point(|pin| pin.position < end);
    points[first..last]
        .iter()
        .find(|pin| match pin.kind {
            PinKind::Clobber { result } => {
                let defines_value = pin.position == fragment_start && result == Some(value);
                !defines_value && !dies_at_fixed_use(value, pin.position, instructions, liveness)
            }
            PinKind::Input(input) => {
                input != VirtualValue::Reg(value)
                    && liveness.after[pin.position]
                        .binary_search_by_key(&value.0, |v| v.0)
                        .is_ok()
                    && !instructions[pin.position]
                        .outputs
                        .iter()
                        .any(|output| output.value == value)
            }
        })
        .map(|pin| pin.position)
        .unwrap_or(usize::MAX)
}

/// A value read at `position` for the last time, only before any output is
/// written: by a Phi copy or as early inputs.
fn dies_at_early_read(
    value: VirtualReg,
    position: usize,
    instructions: &[Instruction],
    liveness: &liveness::Liveness,
) -> bool {
    let instruction = &instructions[position];
    let early = match &instruction.operation {
        Operation::Parallel(_) => true,
        _ => instruction
            .inputs
            .iter()
            .filter(|input| input.value == VirtualValue::Reg(value))
            .all(|input| matches!(input.phase, Phase::Early)),
    };
    early
        && !instruction.facts().definitions.contains(&value)
        && liveness.after[position]
            .binary_search_by_key(&value.0, |v| v.0)
            .is_err()
}

/// A value whose last reads at `position` are early fixed inputs is consumed
/// before the instruction clobbers registers or writes a fixed result, so
/// neither ends its register range; any other read would be copied out of a
/// clobbered register.
fn dies_at_fixed_use(
    value: VirtualReg,
    position: usize,
    instructions: &[Instruction],
    liveness: &liveness::Liveness,
) -> bool {
    let instruction = &instructions[position];
    let mut reads = instruction
        .inputs
        .iter()
        .filter(|input| input.value == VirtualValue::Reg(value))
        .peekable();
    reads.peek().is_some()
        && reads.all(|input| {
            matches!(
                (input.constraint, input.phase),
                (Constraint::Fixed(_), Phase::Early)
            )
        })
        && instruction
            .outputs
            .iter()
            .all(|output| output.value != value)
        && liveness.after[position]
            .binary_search_by_key(&value.0, |v| v.0)
            .is_err()
}

#[cfg(test)]
#[path = "fixed_input_tests.rs"]
mod fixed_input_tests;
