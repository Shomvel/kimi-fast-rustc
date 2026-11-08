use kimi_fast_rustc::regalloc::{
    AllocationResult, LiveInterval, Location, MoveHint, PhysicalRegister, RegAllocError,
    RegisterAllocator, SpillSlot, VirtualReg,
};

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

fn allocate(
    register_count: u16,
    intervals: &[LiveInterval],
    hints: &[MoveHint],
) -> AllocationResult {
    RegisterAllocator::new(register_count)
        .allocate_with_move_hints(intervals, hints)
        .unwrap()
}

fn remaining_register_moves(result: &AllocationResult, hint: MoveHint) -> usize {
    let destination = result.allocation(hint.destination).unwrap();
    let source = result.allocation(hint.source).unwrap();
    usize::from(destination.register().is_some() && destination != source)
}

#[test]
fn exact_move_boundary_eliminates_a_real_register_move() {
    let intervals = [interval(0, 0, 2), interval(1, 0, 5), interval(2, 1, 4)];
    let hint = MoveHint::new(1, vreg(2), vreg(0));
    let baseline = allocate(3, &intervals, &[]);
    let preferred = allocate(3, &intervals, &[hint]);

    assert_eq!(
        baseline.allocation(vreg(2)),
        Some(Location::Register(register(2)))
    );
    assert_eq!(
        preferred.allocation(vreg(2)),
        Some(Location::Register(register(0)))
    );
    assert_eq!(remaining_register_moves(&baseline, hint), 1);
    assert_eq!(remaining_register_moves(&preferred, hint), 0);
    assert_eq!(baseline.spill_slot_count, preferred.spill_slot_count);
}

#[test]
fn a_move_chain_eliminates_all_eight_register_moves() {
    let intervals = (0..=8)
        .map(|number| interval(number, number, number + 2))
        .collect::<Vec<_>>();
    let hints = (0..8)
        .map(|source| MoveHint::new(source + 1, vreg(source + 1), vreg(source)))
        .collect::<Vec<_>>();
    let baseline = allocate(2, &intervals, &[]);
    let preferred = allocate(2, &intervals, &hints);
    let count_moves = |result: &AllocationResult| {
        hints
            .iter()
            .map(|hint| remaining_register_moves(result, *hint))
            .sum::<usize>()
    };

    assert_eq!(count_moves(&baseline), 8);
    assert_eq!(count_moves(&preferred), 0);
    assert_eq!(preferred.spill_slot_count, 0);
}

#[test]
fn coalescing_can_avoid_spilling_an_unrelated_interval() {
    let intervals = [interval(0, 0, 2), interval(1, 0, 5), interval(2, 1, 4)];
    let hint = MoveHint::new(1, vreg(2), vreg(0));
    let baseline = allocate(2, &intervals, &[]);
    let preferred = allocate(2, &intervals, &[hint]);

    assert_eq!(
        baseline.allocation(vreg(1)),
        Some(Location::Spill(SpillSlot(0)))
    );
    assert_eq!(
        preferred.allocation(vreg(1)),
        Some(Location::Register(register(1)))
    );
    assert_eq!(
        preferred.allocation(vreg(2)),
        Some(Location::Register(register(0)))
    );
    assert_eq!(baseline.spill_slot_count, 1);
    assert_eq!(preferred.spill_slot_count, 0);
}

#[test]
fn source_that_survives_the_move_is_not_coalesced() {
    let intervals = [interval(0, 0, 3), interval(1, 1, 4)];
    let hint = MoveHint::new(1, vreg(1), vreg(0));
    let baseline = allocate(2, &intervals, &[]);
    let preferred = allocate(2, &intervals, &[hint]);

    assert_eq!(preferred, baseline);
    assert_ne!(preferred.allocation(vreg(0)), preferred.allocation(vreg(1)));
}

#[test]
fn hint_position_must_be_the_destinations_first_definition() {
    let intervals = [interval(0, 0, 3), interval(1, 1, 4)];
    let hint = MoveHint::new(2, vreg(1), vreg(0));

    assert_eq!(
        allocate(2, &intervals, &[hint]),
        allocate(2, &intervals, &[])
    );
}

#[test]
fn fixed_destination_register_takes_precedence_over_a_hint() {
    let intervals = [interval(0, 0, 2), fixed(1, 1, 4, 1)];
    let hint = MoveHint::new(1, vreg(1), vreg(0));
    let result = allocate(2, &intervals, &[hint]);

    assert_eq!(
        result.allocation(vreg(0)),
        Some(Location::Register(register(0)))
    );
    assert_eq!(
        result.allocation(vreg(1)),
        Some(Location::Register(register(1)))
    );
}

#[test]
fn fixed_source_register_can_be_inherited() {
    let intervals = [fixed(0, 0, 2, 1), interval(1, 1, 4)];
    let hint = MoveHint::new(1, vreg(1), vreg(0));
    let result = allocate(2, &intervals, &[hint]);

    assert_eq!(
        result.allocation(vreg(1)),
        Some(Location::Register(register(1)))
    );
}

#[test]
fn another_definition_at_the_move_disables_coalescing() {
    let intervals = [fixed(0, 0, 2, 0), interval(1, 1, 2), fixed(2, 1, 3, 0)];
    let hint = MoveHint::new(1, vreg(1), vreg(0));
    let error = RegisterAllocator::new(1)
        .allocate_with_move_hints(&intervals, &[hint])
        .unwrap_err();

    assert_eq!(
        error,
        RegAllocError::ConflictingFixedRegisters {
            virtual_reg: vreg(2),
            other_virtual_reg: vreg(0),
            register: register(0),
        }
    );
}

#[test]
fn a_coalesced_destination_can_spill_after_the_source_expires() {
    let intervals = [interval(0, 0, 2), interval(1, 1, 10), fixed(2, 2, 3, 0)];
    let hint = MoveHint::new(1, vreg(1), vreg(0));
    let result = allocate(1, &intervals, &[hint]);

    assert_eq!(
        result.allocation(vreg(0)),
        Some(Location::Register(register(0)))
    );
    assert_eq!(
        result.allocation(vreg(1)),
        Some(Location::Spill(SpillSlot(0)))
    );
    assert_eq!(
        result.allocation(vreg(2)),
        Some(Location::Register(register(0)))
    );
}

#[test]
fn spilled_sources_are_not_forced_back_into_registers() {
    let intervals = [interval(0, 0, 2), interval(1, 1, 4)];
    let hint = MoveHint::new(1, vreg(1), vreg(0));

    assert_eq!(
        allocate(0, &intervals, &[hint]),
        allocate(0, &intervals, &[])
    );
}

#[test]
fn phi_back_edge_does_not_redefine_the_intervals_first_location() {
    let intervals = [interval(0, 0, 2), interval(1, 1, 8), interval(2, 4, 6)];
    let initial = MoveHint::new(1, vreg(1), vreg(0));
    let back_edge = MoveHint::new(5, vreg(1), vreg(2));
    let result = allocate(2, &intervals, &[initial, back_edge]);

    assert_eq!(
        result.allocation(vreg(1)),
        Some(Location::Register(register(0)))
    );
    assert_eq!(
        result.allocation(vreg(2)),
        Some(Location::Register(register(1)))
    );
    assert_eq!(remaining_register_moves(&result, initial), 0);
    assert_eq!(remaining_register_moves(&result, back_edge), 1);
}

#[test]
fn unknown_self_and_duplicate_hints_are_deterministic_and_safe() {
    let intervals = [interval(0, 0, 2), interval(1, 0, 5), interval(2, 1, 4)];
    let preferred_hint = MoveHint::new(1, vreg(2), vreg(0));
    let hints = [
        MoveHint::new(1, vreg(99), vreg(0)),
        MoveHint::new(0, vreg(0), vreg(0)),
        preferred_hint,
        preferred_hint,
    ];
    let expected = allocate(3, &intervals, &[preferred_hint]);

    for offset in 0..hints.len() {
        let mut rotated = hints[offset..].to_vec();
        rotated.extend_from_slice(&hints[..offset]);
        rotated.reverse();
        assert_eq!(allocate(3, &intervals, &rotated), expected);
    }
}
