//! Recreate independent, flag-preserving recipes instead of reloading spills.
//!
//! A candidate spill slot must have one identical recipe at every store. A
//! separate CFG proof rejects every slot that can be read before a definition.
//! This deliberately does not recreate expressions with live virtual inputs,
//! thread-local addresses, memory reads, or aligned addresses requiring AND.
//! The emitter owns the first `spill_slots` slots; addressable source locals
//! are laid out after them. Their storage and the frame layout stay unchanged.
use super::*;
use std::collections::{BTreeMap, VecDeque};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Recipe {
    Immediate(i64),
    Frame(i32),
    Symbol(u32, i64),
}

impl Recipe {
    fn instruction(self, dst: Reg) -> MachineInstr {
        match self {
            Self::Immediate(value) => MachineInstr::MovImm { dst, value },
            Self::Frame(offset) => MachineInstr::StackAddress { dst, offset },
            Self::Symbol(symbol, addend) => MachineInstr::SymbolAddress {
                dst,
                symbol,
                addend,
            },
        }
    }
}

#[derive(Default)]
struct Slot {
    recipe: Option<Recipe>,
    rejected: bool,
    loads: usize,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Stats {
    pub slots: usize,
    pub loads_recreated: usize,
    pub stores_removed: usize,
    pub visits: usize,
    pub budget_bailouts: usize,
}

struct Work {
    left: usize,
    visits: usize,
}

impl Work {
    fn charge(&mut self, amount: usize) -> Option<()> {
        let paid = amount.min(self.left);
        self.left -= paid;
        self.visits += paid;
        (amount == paid).then_some(())
    }
}

pub(super) fn run(function: &mut MachineFunction, spill_slots: u32) -> Stats {
    run_with_budget(function, spill_slots, 262_144)
}

fn run_with_budget(function: &mut MachineFunction, spill_slots: u32, budget: usize) -> Stats {
    let mut stats = Stats::default();
    if spill_slots == 0 || function.instructions.is_empty() {
        return stats;
    }
    let mut work = Work {
        left: budget,
        visits: 0,
    };
    // Plan from one immutable snapshot. Exhaustion never publishes a partial
    // rewrite, including stores whose dependent loads were not yet checked.
    let plan = plan(&function.instructions, spill_slots, &mut work);
    stats.visits = work.visits;
    let Some(recipes) = plan else {
        stats.budget_bailouts = 1;
        return stats;
    };
    stats.slots = recipes.len();
    if recipes.is_empty() {
        return stats;
    }
    function.instructions.retain_mut(|instruction| {
        match *instruction {
            MachineInstr::Load { dst, slot } => {
                if let Some(recipe) = recipes.get(&slot.0) {
                    *instruction = recipe.instruction(dst);
                    stats.loads_recreated += 1;
                }
            }
            MachineInstr::Store { slot, .. } if recipes.contains_key(&slot.0) => {
                stats.stores_removed += 1;
                return false;
            }
            _ => {}
        }
        true
    });
    stats
}

fn stack_operand(instruction: MachineInstr) -> Option<StackSlot> {
    use MachineInstr::*;
    let operand = match instruction {
        Add { src, .. }
        | Sub { src, .. }
        | IMul { src, .. }
        | And { src, .. }
        | Or { src, .. }
        | Xor { src, .. }
        | IDiv { src }
        | UDiv { src }
        | Cmp { rhs: src, .. } => src,
        _ => return None,
    };
    match operand {
        MachineOperand::Stack(slot) => Some(slot),
        _ => None,
    }
}

fn plan(
    instructions: &[MachineInstr],
    spill_slots: u32,
    work: &mut Work,
) -> Option<BTreeMap<u32, Recipe>> {
    let mut registers = [None; 16];
    let mut slots = BTreeMap::<u32, Slot>::new();
    for &instruction in instructions {
        work.charge(1)?;
        if let Some(slot) = stack_operand(instruction)
            && slot.0 < spill_slots
        {
            slots.entry(slot.0).or_default().rejected = true;
        }
        use MachineInstr::*;
        match instruction {
            MovImm { dst, value } => {
                // A movabs may be substantially larger than a reload. Leave
                // those constants to the use-site immediate selector.
                registers[dst as usize] =
                    i32::try_from(value).ok().map(|_| Recipe::Immediate(value));
            }
            StackAddress { dst, offset } => {
                registers[dst as usize] = (i64::from(offset) < -i64::from(spill_slots) * 8)
                    .then_some(Recipe::Frame(offset));
            }
            SymbolAddress {
                dst,
                symbol,
                addend,
            } => registers[dst as usize] = Some(Recipe::Symbol(symbol, addend)),
            Mov { dst, src } => registers[dst as usize] = registers[src as usize],
            Load { dst, slot } => {
                registers[dst as usize] = None;
                if slot.0 < spill_slots {
                    slots.entry(slot.0).or_default().loads += 1;
                }
            }
            Store { slot, src } if slot.0 < spill_slots => {
                let slot = slots.entry(slot.0).or_default();
                match registers[src as usize] {
                    Some(recipe) if slot.recipe.is_none_or(|old| old == recipe) => {
                        slot.recipe = Some(recipe);
                    }
                    _ => slot.rejected = true,
                }
            }
            Store { .. }
            | StoreArg { .. }
            | StoreIndirect { .. }
            | StoreImmediate { .. }
            | StoreAddress { .. }
            | StoreAddressImmediate { .. }
            | StoreVector { .. }
            | LoadVector { .. }
            | Cmp { .. }
            | Test { .. }
            | TestByte { .. }
            | TestMasked { .. } => {}
            // Only propagate recipes within straight-line copy sequences.
            // Clearing everything also covers all implicit definitions, calls
            // and merge points without relying on a target clobber table.
            _ => registers.fill(None),
        }
    }
    let candidates: Vec<_> = slots
        .into_iter()
        .filter_map(|(slot, facts)| {
            (!facts.rejected && facts.loads != 0)
                .then_some(facts.recipe.map(|recipe| (slot, recipe)))
                .flatten()
        })
        .take(64)
        .collect();
    if candidates.is_empty() {
        return Some(BTreeMap::new());
    }

    let mut labels = BTreeMap::new();
    for (position, &instruction) in instructions.iter().enumerate() {
        work.charge(1)?;
        if let MachineInstr::Label(label) = instruction
            && labels.insert(label, position).is_some()
        {
            return None;
        }
    }
    let indices: BTreeMap<_, _> = candidates
        .iter()
        .enumerate()
        .map(|(index, &(slot, _))| (slot, index))
        .collect();
    // Track paths on which each slot has not yet been initialized. The meet
    // is union; a bit is removed only by an actual store on that path. This
    // finds missing stores through joins, loop entries and irreducible CFGs.
    let all = u64::MAX >> (64 - candidates.len());
    let mut before = vec![0u64; instructions.len()];
    let mut reached = vec![false; instructions.len()];
    let mut queued = vec![false; instructions.len()];
    let mut pending = VecDeque::from([0usize]);
    before[0] = all;
    reached[0] = true;
    queued[0] = true;
    let mut rejected = 0u64;
    while let Some(position) = pending.pop_front() {
        work.charge(1)?;
        queued[position] = false;
        let mut missing = before[position];
        if let MachineInstr::Load { slot, .. } = instructions[position]
            && let Some(&index) = indices.get(&slot.0)
        {
            rejected |= missing & (1u64 << index);
        }
        if let MachineInstr::Store { slot, .. } = instructions[position]
            && let Some(&index) = indices.get(&slot.0)
        {
            missing &= !(1u64 << index);
        }
        let mut successors = [None, None];
        match instructions[position] {
            MachineInstr::Ret => {}
            MachineInstr::Jmp(label) => successors[0] = Some(*labels.get(&label)?),
            MachineInstr::Jcc { target, .. } => {
                successors[0] = Some(*labels.get(&target)?);
                successors[1] = (position + 1 < instructions.len()).then_some(position + 1);
            }
            _ => successors[0] = (position + 1 < instructions.len()).then_some(position + 1),
        }
        for target in successors.into_iter().flatten() {
            work.charge(1)?;
            let changed = !reached[target] || missing & !before[target] != 0;
            reached[target] = true;
            before[target] |= missing;
            if changed && !queued[target] {
                pending.push_back(target);
                queued[target] = true;
            }
        }
    }
    Some(
        candidates
            .into_iter()
            .enumerate()
            .filter(|(index, _)| rejected & (1u64 << index) == 0)
            .map(|(_, entry)| entry)
            .collect(),
    )
}

#[cfg(test)]
mod tests;
