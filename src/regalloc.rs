//! Target-independent linear-scan register allocation.
//!
//! Live intervals use half-open instruction ranges: `[start, end)`. Consequently,
//! one interval ending at an instruction does not overlap another starting there.
//! Each virtual register receives one location for its whole interval; spilling an
//! active interval therefore also applies to its already-allocated prefix.

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct VirtualReg(pub u32);

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct InstructionPosition(pub u32);

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct PhysicalRegister(pub u16);

impl PhysicalRegister {
    pub fn index(self) -> usize {
        usize::from(self.0)
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct SpillSlot(pub u32);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LiveInterval {
    pub virtual_reg: VirtualReg,
    pub start: InstructionPosition,
    pub end: InstructionPosition,
    pub fixed_register: Option<PhysicalRegister>,
}

impl LiveInterval {
    pub fn new(virtual_reg: VirtualReg, start: u32, end: u32) -> Self {
        Self {
            virtual_reg,
            start: InstructionPosition(start),
            end: InstructionPosition(end),
            fixed_register: None,
        }
    }

    pub fn fixed(
        virtual_reg: VirtualReg,
        start: u32,
        end: u32,
        register: PhysicalRegister,
    ) -> Self {
        Self::new(virtual_reg, start, end).with_fixed_register(register)
    }

    pub fn with_fixed_register(mut self, register: PhysicalRegister) -> Self {
        self.fixed_register = Some(register);
        self
    }

    pub fn overlaps(&self, other: &Self) -> bool {
        self.start < other.end && other.start < self.end
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MoveHint {
    pub position: InstructionPosition,
    pub destination: VirtualReg,
    pub source: VirtualReg,
}

impl MoveHint {
    pub fn new(position: u32, destination: VirtualReg, source: VirtualReg) -> Self {
        Self {
            position: InstructionPosition(position),
            destination,
            source,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Location {
    Register(PhysicalRegister),
    Spill(SpillSlot),
}

impl Location {
    pub fn register(self) -> Option<PhysicalRegister> {
        match self {
            Self::Register(register) => Some(register),
            Self::Spill(_) => None,
        }
    }

    pub fn spill_slot(self) -> Option<SpillSlot> {
        match self {
            Self::Register(_) => None,
            Self::Spill(slot) => Some(slot),
        }
    }

    pub fn is_spill(self) -> bool {
        matches!(self, Self::Spill(_))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AllocationResult {
    /// Final locations, ordered by virtual register for deterministic inspection.
    pub allocations: BTreeMap<VirtualReg, Location>,
    /// Number of distinct slots created. Expired slots may be reused.
    pub spill_slot_count: u32,
}

impl AllocationResult {
    pub fn allocation(&self, virtual_reg: VirtualReg) -> Option<Location> {
        self.allocations.get(&virtual_reg).copied()
    }

    pub fn iter(&self) -> impl Iterator<Item = (VirtualReg, Location)> + '_ {
        self.allocations
            .iter()
            .map(|(&vreg, &location)| (vreg, location))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RegAllocError {
    EmptyInterval {
        virtual_reg: VirtualReg,
        start: InstructionPosition,
        end: InstructionPosition,
    },
    DuplicateVirtualReg(VirtualReg),
    FixedRegisterOutOfRange {
        virtual_reg: VirtualReg,
        register: PhysicalRegister,
        register_count: u16,
    },
    ConflictingFixedRegisters {
        virtual_reg: VirtualReg,
        other_virtual_reg: VirtualReg,
        register: PhysicalRegister,
    },
    SpillSlotExhausted,
}

impl fmt::Display for RegAllocError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyInterval {
                virtual_reg,
                start,
                end,
            } => write!(
                formatter,
                "virtual register {} has an empty or inverted interval [{}, {})",
                virtual_reg.0, start.0, end.0
            ),
            Self::DuplicateVirtualReg(vreg) => {
                write!(formatter, "duplicate virtual register {}", vreg.0)
            }
            Self::FixedRegisterOutOfRange {
                virtual_reg,
                register,
                register_count,
            } => write!(
                formatter,
                "virtual register {} requires physical register {}, but only {} registers exist",
                virtual_reg.0, register.0, register_count
            ),
            Self::ConflictingFixedRegisters {
                virtual_reg,
                other_virtual_reg,
                register,
            } => write!(
                formatter,
                "virtual registers {} and {} overlap with incompatible constraints on physical register {}",
                virtual_reg.0, other_virtual_reg.0, register.0
            ),
            Self::SpillSlotExhausted => write!(formatter, "spill slot index space exhausted"),
        }
    }
}

impl Error for RegAllocError {}

#[derive(Clone, Debug)]
pub struct RegisterAllocator {
    register_count: u16,
}

impl RegisterAllocator {
    /// Creates an allocator for physical registers `0..register_count`.
    /// A zero-register allocator is valid and spills every interval.
    pub fn new(register_count: u16) -> Self {
        Self { register_count }
    }

    pub fn register_count(&self) -> u16 {
        self.register_count
    }

    pub fn allocate(&self, intervals: &[LiveInterval]) -> Result<AllocationResult, RegAllocError> {
        self.allocate_with_move_hints(intervals, &[])
    }

    /// Allocates registers, coalescing moves whose interval boundaries prove that the
    /// source dies at the move and the destination is first defined there. Coalescing
    /// also requires the destination to be the only interval starting at that position.
    /// Hints that do not satisfy these checks are ignored.
    pub fn allocate_with_move_hints(
        &self,
        intervals: &[LiveInterval],
        move_hints: &[MoveHint],
    ) -> Result<AllocationResult, RegAllocError> {
        let mut intervals = intervals.to_vec();
        intervals.sort_by_key(|interval| (interval.start, interval.end, interval.virtual_reg));

        let mut sorted_hints = move_hints.to_vec();
        sorted_hints.sort_by_key(|hint| (hint.destination, hint.position, hint.source));
        let mut hints = BTreeMap::<VirtualReg, BTreeMap<InstructionPosition, VirtualReg>>::new();
        for hint in sorted_hints {
            hints
                .entry(hint.destination)
                .or_default()
                .entry(hint.position)
                .or_insert(hint.source);
        }

        let mut seen = BTreeSet::new();
        let mut start_counts = BTreeMap::new();
        for interval in &intervals {
            if interval.start >= interval.end {
                return Err(RegAllocError::EmptyInterval {
                    virtual_reg: interval.virtual_reg,
                    start: interval.start,
                    end: interval.end,
                });
            }
            if !seen.insert(interval.virtual_reg) {
                return Err(RegAllocError::DuplicateVirtualReg(interval.virtual_reg));
            }
            if let Some(register) = interval.fixed_register {
                if register.0 >= self.register_count {
                    return Err(RegAllocError::FixedRegisterOutOfRange {
                        virtual_reg: interval.virtual_reg,
                        register,
                        register_count: self.register_count,
                    });
                }
            }
            *start_counts.entry(interval.start).or_insert(0_usize) += 1;
        }

        let mut result = AllocationResult {
            allocations: BTreeMap::new(),
            spill_slot_count: 0,
        };
        let mut free_registers: BTreeSet<_> =
            (0..self.register_count).map(PhysicalRegister).collect();
        let mut free_spill_slots = BTreeSet::new();
        let mut active: Vec<ActiveInterval> = Vec::new();
        let mut spilled: BTreeSet<SpilledInterval> = BTreeSet::new();

        for interval in intervals {
            active.retain(|entry| {
                if entry.interval.end <= interval.start {
                    free_registers.insert(entry.register);
                    false
                } else {
                    true
                }
            });
            while spilled
                .first()
                .is_some_and(|entry| entry.end <= interval.start)
            {
                let entry = spilled.pop_first().unwrap();
                free_spill_slots.insert(entry.slot);
            }

            if let Some(register) = interval.fixed_register {
                let occupant = active.iter().position(|entry| entry.register == register);
                if let Some(index) = occupant {
                    if active[index].interval.fixed_register == Some(register) {
                        return Err(RegAllocError::ConflictingFixedRegisters {
                            virtual_reg: interval.virtual_reg,
                            other_virtual_reg: active[index].interval.virtual_reg,
                            register,
                        });
                    }
                    spill_active(
                        index,
                        &mut active,
                        &mut spilled,
                        &mut result,
                        &mut free_spill_slots,
                    )?;
                } else {
                    let was_free = free_registers.remove(&register);
                    debug_assert!(was_free, "fixed register was neither active nor free");
                }
                result
                    .allocations
                    .insert(interval.virtual_reg, Location::Register(register));
                active.push(ActiveInterval { interval, register });
                continue;
            }

            if start_counts.get(&interval.start) == Some(&1) {
                let source = hints
                    .get(&interval.virtual_reg)
                    .and_then(|positions| positions.get(&interval.start))
                    .copied();
                if let Some(source) = source {
                    let source_end = interval.start.0.checked_add(1).map(InstructionPosition);
                    let source_index = source_end.and_then(|end| {
                        active.iter().position(|entry| {
                            entry.interval.virtual_reg == source && entry.interval.end == end
                        })
                    });
                    if let Some(index) = source_index {
                        let register = active.remove(index).register;
                        result
                            .allocations
                            .insert(interval.virtual_reg, Location::Register(register));
                        active.push(ActiveInterval { interval, register });
                        continue;
                    }
                }
            }

            if let Some(register) = free_registers.pop_first() {
                result
                    .allocations
                    .insert(interval.virtual_reg, Location::Register(register));
                active.push(ActiveInterval { interval, register });
                continue;
            }

            // Prefer evicting the active interval with the latest end. Ties use the
            // greatest virtual register number. The incoming interval wins ties so
            // an equal-length active interval is not spilled unnecessarily.
            let spill_candidate = active
                .iter()
                .enumerate()
                .filter(|(_, entry)| entry.interval.fixed_register.is_none())
                .max_by_key(|(_, entry)| (entry.interval.end, entry.interval.virtual_reg))
                .map(|(index, _)| index);

            match spill_candidate {
                Some(index) if active[index].interval.end > interval.end => {
                    let register = active[index].register;
                    spill_active(
                        index,
                        &mut active,
                        &mut spilled,
                        &mut result,
                        &mut free_spill_slots,
                    )?;
                    result
                        .allocations
                        .insert(interval.virtual_reg, Location::Register(register));
                    active.push(ActiveInterval { interval, register });
                }
                _ => {
                    let slot = take_spill_slot(&mut result, &mut free_spill_slots)?;
                    result
                        .allocations
                        .insert(interval.virtual_reg, Location::Spill(slot));
                    spilled.insert(SpilledInterval {
                        end: interval.end,
                        slot,
                    });
                }
            }
        }

        Ok(result)
    }
}

#[derive(Clone, Copy, Debug)]
struct ActiveInterval {
    interval: LiveInterval,
    register: PhysicalRegister,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct SpilledInterval {
    end: InstructionPosition,
    slot: SpillSlot,
}

fn take_spill_slot(
    result: &mut AllocationResult,
    free_spill_slots: &mut BTreeSet<SpillSlot>,
) -> Result<SpillSlot, RegAllocError> {
    if let Some(slot) = free_spill_slots.pop_first() {
        return Ok(slot);
    }
    let slot = SpillSlot(result.spill_slot_count);
    result.spill_slot_count = result
        .spill_slot_count
        .checked_add(1)
        .ok_or(RegAllocError::SpillSlotExhausted)?;
    Ok(slot)
}

fn spill_active(
    index: usize,
    active: &mut Vec<ActiveInterval>,
    spilled: &mut BTreeSet<SpilledInterval>,
    result: &mut AllocationResult,
    free_spill_slots: &mut BTreeSet<SpillSlot>,
) -> Result<(), RegAllocError> {
    let entry = active.remove(index);
    let slot = take_spill_slot(result, free_spill_slots)?;
    result
        .allocations
        .insert(entry.interval.virtual_reg, Location::Spill(slot));
    spilled.insert(SpilledInterval {
        end: entry.interval.end,
        slot,
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vreg(number: u32) -> VirtualReg {
        VirtualReg(number)
    }

    fn register(number: u16) -> PhysicalRegister {
        PhysicalRegister(number)
    }

    fn interval(number: u32, start: u32, end: u32) -> LiveInterval {
        LiveInterval::new(vreg(number), start, end)
    }

    fn fixed(number: u32, start: u32, end: u32, register: u16) -> LiveInterval {
        LiveInterval::fixed(vreg(number), start, end, PhysicalRegister(register))
    }

    fn allocate(register_count: u16, intervals: &[LiveInterval]) -> AllocationResult {
        RegisterAllocator::new(register_count)
            .allocate(intervals)
            .unwrap()
    }

    #[test]
    fn non_overlapping_intervals_reuse_the_lowest_register() {
        let result = allocate(
            2,
            &[interval(0, 0, 3), interval(1, 3, 5), interval(2, 5, 8)],
        );

        assert_eq!(result.spill_slot_count, 0);
        for vreg in [vreg(0), vreg(1), vreg(2)] {
            assert_eq!(
                result.allocation(vreg),
                Some(Location::Register(register(0)))
            );
        }
    }

    #[test]
    fn overlapping_intervals_use_distinct_registers() {
        let result = allocate(
            2,
            &[interval(0, 0, 8), interval(1, 2, 4), interval(2, 4, 6)],
        );

        assert_eq!(
            result.allocation(vreg(0)),
            Some(Location::Register(register(0)))
        );
        assert_eq!(
            result.allocation(vreg(1)),
            Some(Location::Register(register(1)))
        );
        assert_eq!(
            result.allocation(vreg(2)),
            Some(Location::Register(register(1)))
        );
    }

    #[test]
    fn fixed_register_constraint_is_honored() {
        let result = allocate(
            3,
            &[fixed(4, 0, 4, 2), interval(1, 1, 2), interval(2, 2, 3)],
        );

        assert_eq!(
            result.allocation(vreg(4)),
            Some(Location::Register(register(2)))
        );
        assert_eq!(
            result.allocation(vreg(1)),
            Some(Location::Register(register(0)))
        );
        assert_eq!(
            result.allocation(vreg(2)),
            Some(Location::Register(register(0)))
        );
    }

    #[test]
    fn fixed_interval_evicts_an_unconstrained_occupant() {
        let result = allocate(
            2,
            &[
                interval(0, 0, 10),
                interval(1, 1, 10),
                fixed(2, 2, 3, 0),
                interval(3, 4, 5),
            ],
        );

        assert_eq!(
            result.allocation(vreg(0)),
            Some(Location::Spill(SpillSlot(0)))
        );
        assert_eq!(
            result.allocation(vreg(1)),
            Some(Location::Register(register(1)))
        );
        assert_eq!(
            result.allocation(vreg(2)),
            Some(Location::Register(register(0)))
        );
        assert_eq!(
            result.allocation(vreg(3)),
            Some(Location::Register(register(0)))
        );
    }

    #[test]
    fn touching_fixed_intervals_can_share_a_register() {
        let result = allocate(1, &[fixed(0, 0, 4, 0), fixed(1, 4, 7, 0)]);

        assert_eq!(result.spill_slot_count, 0);
        assert_eq!(
            result.allocation(vreg(0)),
            Some(Location::Register(register(0)))
        );
        assert_eq!(
            result.allocation(vreg(1)),
            Some(Location::Register(register(0)))
        );
    }

    #[test]
    fn overlapping_fixed_constraints_are_rejected() {
        let error = RegisterAllocator::new(2)
            .allocate(&[fixed(3, 0, 5, 1), fixed(7, 4, 8, 1)])
            .unwrap_err();

        assert_eq!(
            error,
            RegAllocError::ConflictingFixedRegisters {
                virtual_reg: vreg(7),
                other_virtual_reg: vreg(3),
                register: register(1),
            }
        );
    }

    #[test]
    fn pressure_spills_the_active_interval_with_the_latest_end() {
        let result = allocate(
            2,
            &[interval(0, 0, 20), interval(1, 1, 10), interval(2, 2, 3)],
        );

        assert_eq!(
            result.allocation(vreg(0)),
            Some(Location::Spill(SpillSlot(0)))
        );
        assert_eq!(
            result.allocation(vreg(1)),
            Some(Location::Register(register(1)))
        );
        assert_eq!(
            result.allocation(vreg(2)),
            Some(Location::Register(register(0)))
        );
    }

    #[test]
    fn pressure_spills_the_incoming_interval_when_it_ends_latest() {
        let result = allocate(
            2,
            &[interval(0, 0, 10), interval(1, 1, 20), interval(2, 2, 30)],
        );

        assert_eq!(
            result.allocation(vreg(0)),
            Some(Location::Register(register(0)))
        );
        assert_eq!(
            result.allocation(vreg(1)),
            Some(Location::Register(register(1)))
        );
        assert_eq!(
            result.allocation(vreg(2)),
            Some(Location::Spill(SpillSlot(0)))
        );
    }

    #[test]
    fn incoming_interval_wins_an_end_position_tie() {
        let result = allocate(1, &[interval(0, 0, 10), interval(1, 1, 10)]);

        assert_eq!(
            result.allocation(vreg(0)),
            Some(Location::Register(register(0)))
        );
        assert_eq!(
            result.allocation(vreg(1)),
            Some(Location::Spill(SpillSlot(0)))
        );
    }

    #[test]
    fn active_spill_ties_are_broken_by_virtual_register_number() {
        let result = allocate(
            2,
            &[interval(4, 0, 10), interval(9, 1, 10), interval(6, 2, 9)],
        );

        assert_eq!(
            result.allocation(vreg(4)),
            Some(Location::Register(register(0)))
        );
        assert_eq!(
            result.allocation(vreg(9)),
            Some(Location::Spill(SpillSlot(0)))
        );
        assert_eq!(
            result.allocation(vreg(6)),
            Some(Location::Register(register(1)))
        );
    }

    #[test]
    fn fixed_active_intervals_are_not_selected_for_spilling() {
        let result = allocate(
            2,
            &[fixed(0, 0, 20, 0), fixed(1, 1, 20, 1), interval(2, 2, 3)],
        );

        assert_eq!(
            result.allocation(vreg(0)),
            Some(Location::Register(register(0)))
        );
        assert_eq!(
            result.allocation(vreg(1)),
            Some(Location::Register(register(1)))
        );
        assert_eq!(
            result.allocation(vreg(2)),
            Some(Location::Spill(SpillSlot(0)))
        );
    }

    #[test]
    fn zero_registers_spills_everything_and_reuses_expired_slots() {
        let result = allocate(
            0,
            &[interval(0, 0, 2), interval(1, 1, 2), interval(2, 2, 4)],
        );

        assert_eq!(
            result.allocation(vreg(0)),
            Some(Location::Spill(SpillSlot(0)))
        );
        assert_eq!(
            result.allocation(vreg(1)),
            Some(Location::Spill(SpillSlot(1)))
        );
        assert_eq!(
            result.allocation(vreg(2)),
            Some(Location::Spill(SpillSlot(0)))
        );
        assert_eq!(result.spill_slot_count, 2);
    }

    #[test]
    fn spilled_slots_are_reused_only_after_expiration() {
        let result = allocate(
            1,
            &[
                interval(0, 0, 10),
                interval(1, 1, 20),
                interval(2, 2, 30),
                interval(3, 30, 31),
            ],
        );

        assert_eq!(
            result.allocation(vreg(1)),
            Some(Location::Spill(SpillSlot(0)))
        );
        assert_eq!(
            result.allocation(vreg(2)),
            Some(Location::Spill(SpillSlot(1)))
        );
        assert_eq!(
            result.allocation(vreg(3)),
            Some(Location::Register(register(0)))
        );
        assert_eq!(result.spill_slot_count, 2);
    }

    #[test]
    fn allocation_is_deterministic_for_every_input_rotation() {
        let intervals = [
            interval(8, 7, 30),
            fixed(2, 2, 12, 1),
            interval(5, 0, 20),
            interval(1, 1, 9),
            interval(9, 12, 14),
            interval(3, 3, 25),
        ];
        let expected = allocate(2, &intervals);

        for offset in 0..intervals.len() {
            let mut rotated = intervals[offset..].to_vec();
            rotated.extend_from_slice(&intervals[..offset]);
            rotated.reverse();
            assert_eq!(allocate(2, &rotated), expected);
        }
    }

    #[test]
    fn invalid_intervals_duplicates_and_bad_fixed_registers_are_rejected() {
        let allocator = RegisterAllocator::new(2);
        assert_eq!(
            allocator.allocate(&[interval(4, 3, 3)]),
            Err(RegAllocError::EmptyInterval {
                virtual_reg: vreg(4),
                start: InstructionPosition(3),
                end: InstructionPosition(3),
            })
        );
        assert!(matches!(
            allocator.allocate(&[interval(4, 5, 3)]),
            Err(RegAllocError::EmptyInterval { .. })
        ));
        assert_eq!(
            allocator.allocate(&[interval(4, 0, 2), interval(4, 2, 3)]),
            Err(RegAllocError::DuplicateVirtualReg(vreg(4)))
        );
        assert_eq!(
            allocator.allocate(&[fixed(4, 0, 2, 2)]),
            Err(RegAllocError::FixedRegisterOutOfRange {
                virtual_reg: vreg(4),
                register: register(2),
                register_count: 2,
            })
        );
    }

    #[test]
    fn public_queries_and_overlap_use_the_same_half_open_contract() {
        let first = interval(0, 0, 4);
        let touching = interval(1, 4, 8);
        let overlapping = interval(2, 3, 8);
        assert!(!first.overlaps(&touching));
        assert!(first.overlaps(&overlapping));

        let result = allocate(0, &[first]);
        let location = result.allocation(vreg(0)).unwrap();
        assert_eq!(location.spill_slot(), Some(SpillSlot(0)));
        assert_eq!(location.register(), None);
        assert!(location.is_spill());
        assert_eq!(result.iter().collect::<Vec<_>>(), vec![(vreg(0), location)]);
        assert_eq!(result.allocation(vreg(99)), None);
    }
}
