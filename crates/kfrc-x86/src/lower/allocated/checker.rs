//! Local symbolic proof of every constrained fragment and its boundary copies.
//! Each fragment starts/ends at linear-scan homes. Checking preservation and
//! uses there composes across CFG edges, including simultaneous Phi definitions.
use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Symbol {
    Before(VirtualReg),
    After(VirtualReg),
    Constant(i64),
    Argument(usize),
}
type State = HashMap<Location, Symbol>;
fn enabled() -> bool {
    std::env::var("KFRC_REGALLOC_CHECK").as_deref() != Ok("0")
}
fn failure(message: impl Into<String>) -> LowerError {
    LowerError::Malformed(format!("allocation checker: {}", message.into()))
}
fn read(state: &State, location: Location) -> Result<Symbol, LowerError> {
    if let Location::Imm(n) = location {
        Ok(Symbol::Constant(n))
    } else {
        state
            .get(&location)
            .copied()
            .ok_or_else(|| failure(format!("uninitialized {location:?}")))
    }
}
fn insert(state: &mut State, location: Location, symbol: Symbol) -> Result<(), LowerError> {
    if let Some(old) = state.insert(location, symbol)
        && old != symbol
    {
        return Err(failure(format!(
            "overlapping homes {location:?}: {old:?} / {symbol:?}"
        )));
    }
    Ok(())
}
fn moves(state: &mut State, instructions: &[MachineInstr]) -> Result<(), LowerError> {
    for instruction in instructions {
        use MachineInstr::*;
        let (src, dst) = match instruction {
            Mov { dst, src } => (Location::Reg(*src), Location::Reg(*dst)),
            MovImm { dst, value } => (Location::Imm(*value), Location::Reg(*dst)),
            Load { dst, slot } => (Location::Stack(*slot), Location::Reg(*dst)),
            Store { slot, src } => (Location::Reg(*src), Location::Stack(*slot)),
            _ => return Err(failure("non-copy instruction in boundary moves")),
        };
        // Borrowed scratch registers may initially be dead/unspecified. Their
        // save/restore is still checked when a live symbolic value occupies it.
        let value = match read(state, src) {
            Ok(v) => Some(v),
            Err(_) => None,
        };
        if let Some(value) = value {
            state.insert(dst, value);
        } else {
            state.remove(&dst);
        }
    }
    Ok(())
}
fn expect(state: &State, location: Location, expected: Symbol) -> Result<(), LowerError> {
    let actual = read(state, location)?;
    if actual != expected {
        return Err(failure(format!(
            "{location:?}: expected {expected:?}, got {actual:?}"
        )));
    }
    Ok(())
}
/// The home of `v` and, for a value stored at its definitions, its slot.
fn locations(
    allocation: &allocation::Allocation,
    v: VirtualReg,
    position: usize,
) -> Result<Vec<Location>, LowerError> {
    let home = allocation.home(v, position)?;
    let mut locations = vec![home];
    if let Some(slot) = allocation.memory_slot(v)
        && home != Location::Stack(slot)
    {
        locations.push(Location::Stack(slot));
    }
    Ok(locations)
}
fn expect_all(
    allocation: &allocation::Allocation,
    state: &State,
    v: VirtualReg,
    position: usize,
    expected: Symbol,
) -> Result<(), LowerError> {
    for location in locations(allocation, v, position)? {
        expect(state, location, expected)?;
    }
    Ok(())
}
fn insert_all(
    allocation: &allocation::Allocation,
    state: &mut State,
    v: VirtualReg,
    position: usize,
    symbol: Symbol,
) -> Result<(), LowerError> {
    for location in locations(allocation, v, position)? {
        insert(state, location, symbol)?;
    }
    Ok(())
}
fn initial(
    allocation: &allocation::Allocation,
    inst: &Instruction,
    position: usize,
) -> Result<(State, Vec<VirtualReg>), LowerError> {
    let facts = inst.facts();
    let live = allocation.live_through(position, &facts.definitions);
    let mut state = State::new();
    for v in live.iter().chain(&facts.uses) {
        insert_all(allocation, &mut state, *v, position, Symbol::Before(*v))?;
    }
    Ok((state, live))
}
pub(super) fn instruction(
    allocation: &allocation::Allocation,
    inst: &Instruction,
    position: usize,
    inputs: &[Reg],
    outputs: &[Reg],
    before: &[MachineInstr],
    after: &[MachineInstr],
) -> Result<(), LowerError> {
    if !enabled() {
        return Ok(());
    }
    let (mut state, live) = initial(allocation, inst, position)?;
    moves(&mut state, before)?;
    for (input, &reg) in inst.inputs.iter().zip(inputs) {
        let expected = match input.value {
            VirtualValue::Reg(v) => Symbol::Before(v),
            VirtualValue::Imm(n) => Symbol::Constant(n),
        };
        expect(&state, Location::Reg(reg), expected)?;
        if let Constraint::Fixed(fixed) = input.constraint
            && fixed != reg
        {
            return Err(failure("fixed input mismatch"));
        }
    }
    for &r in &inst.clobbers {
        state.remove(&Location::Reg(r));
    }
    for (output, &reg) in inst.outputs.iter().zip(outputs) {
        match output.constraint {
            Constraint::Fixed(fixed) if fixed != reg => {
                return Err(failure("fixed output mismatch"));
            }
            Constraint::Reuse(index) if inputs[index] != reg => {
                return Err(failure("reuse mismatch"));
            }
            _ => {}
        }
        state.insert(Location::Reg(reg), Symbol::After(output.value));
    }
    moves(&mut state, after)?;
    for v in live {
        expect_all(allocation, &state, v, position, Symbol::Before(v))?;
    }
    for output in &inst.outputs {
        expect_all(
            allocation,
            &state,
            output.value,
            position,
            Symbol::After(output.value),
        )?;
    }
    Ok(())
}
pub(super) fn entry(
    allocation: &allocation::Allocation,
    args: &[(VirtualReg, usize)],
    position: usize,
    instructions: &[MachineInstr],
) -> Result<(), LowerError> {
    if !enabled() {
        return Ok(());
    }
    let mut state = State::new();
    for (_, index) in args {
        state.insert(
            Location::Reg(Reg::ARGUMENTS[*index]),
            Symbol::Argument(*index),
        );
    }
    moves(&mut state, instructions)?;
    for (v, index) in args {
        expect_all(allocation, &state, *v, position, Symbol::Argument(*index))?;
    }
    Ok(())
}
pub(super) fn parallel(
    allocation: &allocation::Allocation,
    inst: &Instruction,
    position: usize,
    instructions: &[MachineInstr],
) -> Result<(), LowerError> {
    if !enabled() {
        return Ok(());
    }
    let Operation::Parallel(copies) = &inst.operation else {
        unreachable!()
    };
    let (mut state, live) = initial(allocation, inst, position)?;
    let expected = copies
        .iter()
        .map(|(dst, src)| Ok((*dst, read(&state, allocation.value(*src, position)?)?)))
        .collect::<Result<Vec<_>, LowerError>>()?;
    moves(&mut state, instructions)?;
    for v in live {
        expect_all(allocation, &state, v, position, Symbol::Before(v))?;
    }
    for (v, symbol) in expected {
        expect_all(allocation, &state, v, position, symbol)?;
    }
    Ok(())
}

pub(super) fn edge(
    allocation: &allocation::Allocation,
    source: usize,
    target: usize,
    instructions: &[MachineInstr],
) -> Result<(), LowerError> {
    if !enabled() {
        return Ok(());
    }
    let mut state = State::new();
    let values = allocation.edge_values(source, target);
    for &v in &values {
        insert_all(allocation, &mut state, v, source, Symbol::Before(v))?;
    }
    moves(&mut state, instructions)?;
    for &v in &values {
        expect_all(allocation, &state, v, target, Symbol::Before(v))?;
    }
    Ok(())
}
