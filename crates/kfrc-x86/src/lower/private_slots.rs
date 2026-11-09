//! Remove exact lane round trips through private, nonescaping frame storage.
//!
//! ABI materialization is already explicit here. All accesses to an accepted
//! slot occupy one straight-line segment; no memory state crosses a branch,
//! label or call. A slot is changed only when every read has a proven writer.
use super::*;

#[derive(Default, Debug, PartialEq, Eq)]
pub(super) struct Stats {
    pub loads_forwarded: usize,
    pub stores_removed: usize,
    pub slots_removed: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Address {
    slot: usize,
    offset: u64,
}

#[derive(Default)]
pub(super) struct Facts {
    pub(super) uses: Vec<VirtualReg>,
    pub(super) defs: Vec<VirtualReg>,
}

impl Facts {
    fn use_value(&mut self, value: VirtualValue) {
        if let VirtualValue::Reg(reg) = value {
            self.uses.push(reg);
        }
    }
    fn target(&mut self, target: &CallDestination) {
        if let CallDestination::Indirect(value) = target {
            self.use_value(*value);
        }
    }
}

// Exhaustive over both instruction enums: additions must declare their data
// dependencies before they can participate in a private-storage proof.
pub(super) fn facts(instruction: &LowerInstr) -> Facts {
    let mut f = Facts::default();
    match instruction {
        LowerInstr::Scalar(i) => match i {
            VirtualInstr::LoadArg { dst, .. }
            | VirtualInstr::Load { dst, .. }
            | VirtualInstr::Phi { dst } => f.defs.push(*dst),
            VirtualInstr::Mov { dst, src }
            | VirtualInstr::Neg { dst, src }
            | VirtualInstr::Not { dst, src } => {
                f.defs.push(*dst);
                f.use_value(*src);
            }
            VirtualInstr::Add { dst, lhs, rhs }
            | VirtualInstr::Sub { dst, lhs, rhs }
            | VirtualInstr::Mul { dst, lhs, rhs }
            | VirtualInstr::And { dst, lhs, rhs }
            | VirtualInstr::Or { dst, lhs, rhs }
            | VirtualInstr::Xor { dst, lhs, rhs }
            | VirtualInstr::Shl { dst, lhs, rhs }
            | VirtualInstr::Shr { dst, lhs, rhs }
            | VirtualInstr::LShr { dst, lhs, rhs }
            | VirtualInstr::Div { dst, lhs, rhs }
            | VirtualInstr::Rem { dst, lhs, rhs }
            | VirtualInstr::UDiv { dst, lhs, rhs }
            | VirtualInstr::URem { dst, lhs, rhs }
            | VirtualInstr::Compare { dst, lhs, rhs, .. } => {
                f.defs.push(*dst);
                f.use_value(*lhs);
                f.use_value(*rhs);
            }
            VirtualInstr::Test { lhs, rhs } => {
                f.use_value(*lhs);
                f.use_value(*rhs);
            }
            VirtualInstr::Store { src, .. } | VirtualInstr::Return(src) => f.use_value(*src),
            VirtualInstr::Call { dst, args, .. } => {
                f.defs.push(*dst);
                for value in args {
                    f.use_value(*value);
                }
            }
            VirtualInstr::Label(_)
            | VirtualInstr::Jmp(_)
            | VirtualInstr::Jcc { .. }
            | VirtualInstr::Nop => {}
        },
        LowerInstr::ParallelCopy(copies) => {
            for (dst, src) in copies {
                f.defs.push(*dst);
                f.use_value(*src);
            }
        }
        LowerInstr::Rotate {
            dst, value, amount, ..
        } => {
            f.defs.push(*dst);
            f.use_value(*value);
            f.use_value(*amount);
        }
        LowerInstr::Extend { dst, value, .. } => {
            f.defs.push(*dst);
            f.uses.push(*value);
        }
        LowerInstr::ByteSwap { dst, value, .. } => {
            f.defs.push(*dst);
            f.use_value(*value);
        }
        LowerInstr::TestLowByte { value } => f.uses.push(*value),
        LowerInstr::BitScanZero {
            dst, value, zero, ..
        } => {
            f.defs.push(*dst);
            f.use_value(*value);
            f.use_value(*zero);
        }
        LowerInstr::StackAddress { dst, .. }
        | LowerInstr::SymbolAddress { dst, .. }
        | LowerInstr::ThreadLocalAddress { dst, .. } => f.defs.push(*dst),
        LowerInstr::Load { dst, address, .. } => {
            f.defs.push(*dst);
            f.use_value(*address);
        }
        LowerInstr::Store { address, src, .. } => {
            f.use_value(*address);
            f.use_value(*src);
        }
        LowerInstr::LoadVector { address, .. } | LowerInstr::StoreVector { address, .. } => {
            f.use_value(*address)
        }
        LowerInstr::MulOverflow { dst, lhs, rhs, .. } => {
            f.defs.push(*dst);
            f.use_value(*lhs);
            f.use_value(*rhs);
        }
        LowerInstr::CallPair {
            first,
            second,
            target,
            args,
        } => {
            f.defs.extend([*first, *second]);
            f.target(target);
            for value in args {
                f.use_value(*value);
            }
        }
        LowerInstr::CallIndirect { dst, target, args } => {
            f.defs.push(*dst);
            f.use_value(*target);
            for value in args {
                f.use_value(*value);
            }
        }
        LowerInstr::AbiCall {
            results,
            target,
            args,
            vector_args,
            vector_result,
        } => {
            f.defs.extend(results);
            f.target(target);
            for (_, value) in args {
                f.use_value(*value);
            }
            for (_, value) in vector_args {
                f.use_value(*value);
            }
            if let Some(value) = vector_result {
                f.use_value(*value);
            }
        }
        LowerInstr::ReturnPair { first, second } => {
            f.use_value(*first);
            f.use_value(*second);
        }
    }
    f
}

fn boundary(i: &LowerInstr) -> bool {
    matches!(
        i,
        LowerInstr::Scalar(
            VirtualInstr::Label(_)
                | VirtualInstr::Jmp(_)
                | VirtualInstr::Jcc { .. }
                | VirtualInstr::Return(_)
                | VirtualInstr::Call { .. }
        ) | LowerInstr::CallPair { .. }
            | LowerInstr::CallIndirect { .. }
            | LowerInstr::AbiCall { .. }
            | LowerInstr::ReturnPair { .. }
    )
}

fn value_address(value: VirtualValue, addresses: &HashMap<VirtualReg, Address>) -> Option<Address> {
    let VirtualValue::Reg(reg) = value else {
        return None;
    };
    addresses.get(&reg).copied()
}

/// The slot address a load/store accesses: its address operand plus displacement.
/// `Some(None)` is an access whose displaced address leaves the slot's offset domain.
fn access_address(
    address: VirtualValue,
    displacement: i32,
    addresses: &HashMap<VirtualReg, Address>,
) -> Option<Option<Address>> {
    let mut address = value_address(address, addresses)?;
    Some(
        address
            .offset
            .checked_add_signed(i64::from(displacement))
            .map(|offset| {
                address.offset = offset;
                address
            }),
    )
}

fn derived(i: &LowerInstr, addresses: &HashMap<VirtualReg, Address>) -> Option<Address> {
    match i {
        LowerInstr::StackAddress { slot, .. } => Some(Address {
            slot: *slot,
            offset: 0,
        }),
        LowerInstr::Scalar(VirtualInstr::Mov { src, .. }) => value_address(*src, addresses),
        LowerInstr::Scalar(VirtualInstr::Add { lhs, rhs, .. }) => {
            let (base, offset) = match (lhs, rhs) {
                (base, VirtualValue::Imm(offset)) | (VirtualValue::Imm(offset), base) => {
                    (*base, *offset)
                }
                _ => return None,
            };
            let mut address = value_address(base, addresses)?;
            address.offset = address.offset.checked_add(u64::try_from(offset).ok()?)?;
            Some(address)
        }
        _ => None,
    }
}

#[derive(Default)]
struct Slot {
    rejected: bool,
    segment: Option<usize>,
    accesses: Vec<usize>,
}

pub(super) fn run(stream: &mut Vec<LowerInstr>, layouts: &mut Vec<SizeAlign>) -> Stats {
    let mut stats = Stats::default();
    // These are a separate, preallocated slot namespace. Do not compact frame
    // indexes in a mixed stream until its remapping contract is explicit.
    if stream.iter().any(|i| {
        matches!(
            i,
            LowerInstr::Scalar(VirtualInstr::Load { .. } | VirtualInstr::Store { .. })
        )
    }) {
        return stats;
    }
    // Graph lowering emits adjacent Test/Jcc pairs. Preserve any other stream
    // with implicit flags live across memory operations; a narrow forwarded
    // load uses And, which must not overwrite a branch's pending flags.
    if stream.iter().enumerate().any(|(index, i)| {
        matches!(i, LowerInstr::Scalar(VirtualInstr::Jcc { .. }))
            && !index.checked_sub(1).is_some_and(|previous| {
                matches!(
                    stream[previous],
                    LowerInstr::Scalar(VirtualInstr::Test { .. }) | LowerInstr::TestLowByte { .. }
                )
            })
    }) {
        return stats;
    }
    let all_facts: Vec<_> = stream.iter().map(facts).collect();
    let mut definitions = HashMap::<VirtualReg, Vec<usize>>::default();
    for (index, f) in all_facts.iter().enumerate() {
        for &reg in &f.defs {
            definitions.entry(reg).or_default().push(index);
        }
    }
    let mut slots: Vec<Slot> = (0..layouts.len()).map(|_| Slot::default()).collect();
    let mut addresses = HashMap::default();
    // A small worklist, rather than textual-order assumptions, also handles a
    // pure address whose single definition was scheduled in another block.
    let mut users = HashMap::<VirtualReg, Vec<usize>>::default();
    for (index, f) in all_facts.iter().enumerate() {
        for &reg in &f.uses {
            users.entry(reg).or_default().push(index);
        }
    }
    let mut pending = Vec::new();
    for (index, i) in stream.iter().enumerate() {
        if let LowerInstr::StackAddress { dst, slot } = i {
            if *slot >= slots.len() {
                return stats;
            }
            if definitions[dst].len() != 1 {
                slots[*slot].rejected = true;
            } else {
                pending.push(index);
            }
        }
    }
    while let Some(index) = pending.pop() {
        let f = &all_facts[index];
        let [dst] = f.defs.as_slice() else { continue };
        if definitions[dst].len() != 1 || addresses.contains_key(dst) {
            continue;
        }
        if let Some(address) = derived(&stream[index], &addresses) {
            addresses.insert(*dst, address);
            pending.extend(users.get(dst).into_iter().flatten().copied());
        }
    }
    let mut segment = 0usize;
    for (index, i) in stream.iter().enumerate() {
        if boundary(i) {
            segment += 1;
        }
        let derived_address = derived(i, &addresses).filter(|_| {
            all_facts[index]
                .defs
                .first()
                .is_some_and(|dst| definitions[dst].len() == 1)
        });
        for reg in &all_facts[index].uses {
            let Some(address) = addresses.get(reg) else {
                continue;
            };
            let permitted = derived_address.is_some()
                || match i {
                    LowerInstr::Load { address: value, .. }
                    | LowerInstr::Store { address: value, .. } => *value == VirtualValue::Reg(*reg),
                    _ => false,
                };
            if !permitted {
                slots[address.slot].rejected = true;
            }
        }
        if let LowerInstr::Load {
            address,
            offset,
            bytes,
            ..
        }
        | LowerInstr::Store {
            address,
            offset,
            bytes,
            ..
        } = i
        {
            let Some(displaced) = access_address(*address, *offset, &addresses) else {
                continue;
            };
            let Some(address) = displaced else {
                let base = value_address(*address, &addresses).unwrap();
                slots[base.slot].rejected = true;
                continue;
            };
            let slot = &mut slots[address.slot];
            if !matches!(bytes, 1 | 2 | 4 | 8)
                || address
                    .offset
                    .checked_add(u64::from(*bytes))
                    .is_none_or(|end| end > layouts[address.slot].size)
                || slot.segment.is_some_and(|old| old != segment)
            {
                slot.rejected = true;
            }
            slot.segment = Some(segment);
            slot.accesses.push(index);
            // Storing the slot's address as data is an escape, even if the
            // same register was also the address operand of this instruction.
            if let LowerInstr::Store { src, .. } = i
                && let Some(source) = value_address(*src, &addresses)
            {
                slots[source.slot].rejected = true;
            }
        }
    }
    let mut versions = HashMap::<VirtualReg, usize>::default();
    let mut at_use = vec![None; stream.len()];
    for (index, i) in stream.iter().enumerate() {
        if let LowerInstr::Store {
            src: VirtualValue::Reg(reg),
            ..
        } = i
        {
            at_use[index] = Some(*versions.get(reg).unwrap_or(&0));
        }
        for &reg in &all_facts[index].defs {
            versions.insert(reg, index + 1);
        }
    }
    let mut removed = vec![false; stream.len()];
    let mut dead_seeds = Vec::new();
    for slot in slots.iter().filter(|slot| !slot.rejected) {
        let mut lanes: Vec<(u64, u8, VirtualValue, Option<usize>)> = Vec::new();
        let mut replacements = Vec::new();
        let mut valid = true;
        for &index in &slot.accesses {
            match stream[index] {
                LowerInstr::Store {
                    address,
                    offset: displacement,
                    src,
                    bytes,
                } => {
                    let offset = access_address(address, displacement, &addresses)
                        .flatten()
                        .unwrap()
                        .offset;
                    lanes.retain(|(old, width, _, _)| {
                        *old + u64::from(*width) <= offset || offset + u64::from(bytes) <= *old
                    });
                    lanes.push((offset, bytes, src, at_use[index]));
                }
                LowerInstr::Load {
                    dst,
                    address,
                    offset: displacement,
                    bytes,
                } => {
                    let offset = access_address(address, displacement, &addresses)
                        .flatten()
                        .unwrap()
                        .offset;
                    let Some((_, _, src, version)) = lanes
                        .iter()
                        .find(|(old, width, _, _)| *old == offset && *width == bytes)
                    else {
                        valid = false;
                        break;
                    };
                    if let VirtualValue::Reg(reg) = src {
                        let latest = definitions
                            .get(reg)
                            .into_iter()
                            .flatten()
                            .copied()
                            .filter(|position| *position < index)
                            .max()
                            .map_or(0, |position| position + 1);
                        if Some(latest) != *version {
                            valid = false;
                            break;
                        }
                    }
                    let replacement = if bytes == 8 {
                        VirtualInstr::Mov { dst, src: *src }
                    } else {
                        VirtualInstr::And {
                            dst,
                            lhs: *src,
                            rhs: VirtualValue::Imm(((1u64 << (u32::from(bytes) * 8)) - 1) as i64),
                        }
                    };
                    replacements.push((index, replacement));
                }
                _ => unreachable!(),
            }
        }
        if !valid {
            continue;
        }
        for &index in &slot.accesses {
            if matches!(stream[index], LowerInstr::Store { .. }) {
                dead_seeds.extend(all_facts[index].uses.iter().copied());
                removed[index] = true;
                stats.stores_removed += 1;
            }
        }
        for (index, replacement) in replacements {
            dead_seeds.extend(all_facts[index].uses.iter().copied());
            stream[index] = LowerInstr::Scalar(replacement);
            stats.loads_forwarded += 1;
        }
    }
    // Lowering can materialize an address even when its load has no value
    // consumer. Seed those address chains too, so an unused slot cannot keep
    // the frame alive merely because no memory access needed replacement.
    dead_seeds.extend(
        addresses
            .iter()
            .filter(|(_, address)| !slots[address.slot].rejected)
            .map(|(&reg, _)| reg),
    );
    clean_dead_values(stream, &mut removed, dead_seeds);
    let mut index = 0;
    stream.retain(|_| {
        let keep = !removed[index];
        index += 1;
        keep
    });
    let mut live = vec![false; layouts.len()];
    for i in stream.iter() {
        if let LowerInstr::StackAddress { slot, .. } = i {
            live[*slot] = true;
        }
    }
    let mut remap = vec![0; layouts.len()];
    let mut next = 0;
    for (slot, &keep) in live.iter().enumerate() {
        if keep {
            remap[slot] = next;
            next += 1;
        }
    }
    stats.slots_removed = layouts.len() - next;
    for i in stream.iter_mut() {
        if let LowerInstr::StackAddress { slot, .. } = i {
            *slot = remap[*slot];
        }
    }
    let mut slot = 0;
    layouts.retain(|_| {
        let keep = live[slot];
        slot += 1;
        keep
    });
    stats
}

fn dead_removable(i: &LowerInstr) -> bool {
    matches!(
        i,
        LowerInstr::StackAddress { .. }
            | LowerInstr::Extend { .. }
            | LowerInstr::Scalar(
                VirtualInstr::LoadArg { .. }
                    | VirtualInstr::Mov { .. }
                    | VirtualInstr::Add { .. }
                    | VirtualInstr::Sub { .. }
                    | VirtualInstr::Mul { .. }
                    | VirtualInstr::And { .. }
                    | VirtualInstr::Or { .. }
                    | VirtualInstr::Xor { .. }
                    | VirtualInstr::Shl { .. }
                    | VirtualInstr::Shr { .. }
                    | VirtualInstr::LShr { .. }
            )
    )
}

pub(super) fn clean_dead_values(
    stream: &[LowerInstr],
    removed: &mut [bool],
    mut pending: Vec<VirtualReg>,
) {
    let all_facts: Vec<_> = stream.iter().map(facts).collect();
    let mut definitions = HashMap::<VirtualReg, Vec<usize>>::default();
    let mut uses = HashMap::<VirtualReg, usize>::default();
    for (index, f) in all_facts.iter().enumerate().filter(|(i, _)| !removed[*i]) {
        for &reg in &f.defs {
            definitions.entry(reg).or_default().push(index);
        }
        for &reg in &f.uses {
            *uses.entry(reg).or_default() += 1;
        }
    }
    while let Some(reg) = pending.pop() {
        if uses.get(&reg).copied().unwrap_or(0) != 0 {
            continue;
        }
        let Some(definitions) = definitions.get(&reg) else {
            continue;
        };
        let [index] = definitions.as_slice() else {
            continue;
        };
        if removed[*index] || !dead_removable(&stream[*index]) {
            continue;
        }
        removed[*index] = true;
        for &input in &all_facts[*index].uses {
            *uses.get_mut(&input).unwrap() -= 1;
            pending.push(input);
        }
    }
}

#[cfg(test)]
#[path = "private_slots_tests.rs"]
mod tests;
