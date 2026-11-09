use super::*;
use crate::machine::eval;

fn reg(reg: Reg) -> Location {
    Location::Reg(reg)
}
fn slot(index: u32) -> Location {
    Location::Stack(StackSlot(index))
}
fn copy(src: Location, dst: Location) -> Copy {
    Copy { src, dst }
}
fn mask(registers: impl IntoIterator<Item = Reg>) -> Protected {
    let mut protected = Protected::default();
    for reg in registers {
        protected.insert(reg);
    }
    protected
}
fn seed(emitter: &mut Emitter<'_>, slots: u32) -> HashMap<Location, i64> {
    emitter.function.stack_slots = slots;
    emitter.stats = Some(ScratchStats::default());
    let mut values = HashMap::default();
    for index in 0..slots {
        let value = 1000 + i64::from(index);
        emitter.push(MachineInstr::MovImm {
            dst: Reg::Rax,
            value,
        });
        emitter.push(MachineInstr::Store {
            slot: StackSlot(index),
            src: Reg::Rax,
        });
        values.insert(slot(index), value);
    }
    for (index, register) in Reg::ALLOCATABLE.into_iter().enumerate() {
        let value = 100 + index as i64;
        emitter.push(MachineInstr::MovImm {
            dst: register,
            value,
        });
        values.insert(reg(register), value);
    }
    values
}

// Compare executed machine copies with simultaneous assignment from the original
// snapshot. Only destinations and protected values are observable; free scratch
// registers may change. This also checks the R11 emergency-save fallback.
fn execute_copies(
    copies: Vec<Copy>,
    protected: Protected,
    slots: u32,
) -> (MachineFunction, ScratchStats) {
    let allocation = allocation::Allocation::new(&[], 0).unwrap();
    let mut emitter = Emitter::new("scratch", &allocation, &[]).unwrap();
    let initial = seed(&mut emitter, slots);
    let mut expected: HashMap<_, _> = Reg::ALLOCATABLE
        .into_iter()
        .filter(|&r| protected.contains(r))
        .map(|r| (reg(r), initial[&reg(r)]))
        .collect();
    for copy in &copies {
        let value = match copy.src {
            Location::Imm(value) => value,
            location => initial[&location],
        };
        expected.insert(copy.dst, value);
    }
    emitter.parallel(copies, protected).unwrap();
    for (location, expected) in expected {
        let mut function = emitter.function.clone();
        match location {
            Location::Reg(src) => function
                .instructions
                .push(MachineInstr::Mov { dst: Reg::Rax, src }),
            Location::Stack(slot) => function.instructions.push(MachineInstr::Load {
                dst: Reg::Rax,
                slot,
            }),
            Location::Imm(_) => unreachable!(),
        }
        function.instructions.push(MachineInstr::Ret);
        assert_eq!(
            eval::evaluate(&function, &[]).unwrap(),
            expected,
            "{location:?}"
        );
    }
    (emitter.function, emitter.stats.unwrap())
}

#[test]
fn register_cycles_and_fanout_use_free_register_without_stack() {
    let (function, stats) = execute_copies(
        vec![
            copy(reg(Reg::Rcx), reg(Reg::Rax)),
            copy(reg(Reg::Rdx), reg(Reg::Rcx)),
            copy(reg(Reg::Rax), reg(Reg::Rdx)),
            copy(reg(Reg::Rax), reg(Reg::R8)),
        ],
        mask([Reg::R11]),
        0,
    );
    assert_eq!(function.stack_slots, 0);
    assert_eq!(stats.cycle_reg, 1);
    assert_eq!(stats.cycle_stack, 0);
    assert_eq!(stats.memory_saved, 0);
}

#[test]
fn identity_and_untouched_live_values_are_not_memory_scratch() {
    let (function, stats) = execute_copies(
        vec![
            copy(reg(Reg::Rax), reg(Reg::Rax)),
            copy(slot(0), slot(1)),
            copy(Location::Imm(0), slot(2)),
        ],
        mask([Reg::Rcx, Reg::R11]),
        3,
    );
    assert_eq!(function.stack_slots, 3);
    assert_eq!(stats.memory_free, 2);
    assert_eq!(stats.memory_saved, 0);
}

#[test]
fn combined_results_and_restores_consume_results_before_overwriting_them() {
    let (function, stats) = execute_copies(
        vec![
            copy(reg(Reg::Rax), reg(Reg::R8)),
            copy(reg(Reg::Rdx), slot(1)),
            copy(reg(Reg::Rcx), reg(Reg::Rcx)),
            copy(slot(0), reg(Reg::Rax)),
        ],
        mask([Reg::Rcx, Reg::R11]),
        2,
    );
    assert_eq!(function.stack_slots, 2);
    assert_eq!(stats.memory_saved, 0);
}

#[test]
fn memory_cycle_preserves_r11_cycle_temporary_through_emergency_scratch() {
    let (function, stats) = execute_copies(
        vec![
            copy(reg(Reg::Rax), reg(Reg::Rcx)),
            copy(reg(Reg::Rcx), reg(Reg::Rax)),
            copy(slot(0), slot(1)),
            copy(slot(1), slot(0)),
        ],
        mask(Reg::ALLOCATABLE.into_iter().filter(|&r| r != Reg::R11)),
        2,
    );
    assert_eq!(stats.cycle_reg, 2);
    assert_eq!(stats.cycle_stack, 0);
    assert!(stats.memory_saved > 0);
    assert_eq!(function.stack_slots, 3, "only the R11 save slot is needed");
}

#[test]
fn full_pressure_reuses_separate_cycle_and_emergency_slots() {
    let (function, stats) = execute_copies(
        vec![
            copy(reg(Reg::Rax), reg(Reg::Rcx)),
            copy(reg(Reg::Rcx), reg(Reg::Rax)),
            copy(slot(0), slot(1)),
            copy(slot(1), slot(0)),
        ],
        mask(Reg::ALLOCATABLE),
        2,
    );
    assert_eq!(stats.cycle_stack, 2);
    assert_eq!(stats.cycle_reg, 0);
    assert!(stats.memory_saved > 0);
    assert_eq!(function.stack_slots, 4);
}

#[test]
fn copy_sequence_preserves_flags_for_following_branch() {
    let allocation = allocation::Allocation::new(&[], 0).unwrap();
    let mut emitter = Emitter::new("scratch_flags", &allocation, &[]).unwrap();
    seed(&mut emitter, 2);
    emitter.push(MachineInstr::Cmp {
        lhs: Reg::Rax,
        rhs: MachineOperand::Imm(100),
    });
    emitter
        .parallel(
            vec![
                copy(Location::Imm(0), slot(0)),
                copy(slot(1), reg(Reg::Rax)),
            ],
            mask(Reg::ALLOCATABLE),
        )
        .unwrap();
    emitter.push(MachineInstr::Jcc {
        condition: Condition::Eq,
        target: 1,
    });
    emitter.push(MachineInstr::MovImm {
        dst: Reg::Rax,
        value: -1,
    });
    emitter.push(MachineInstr::Ret);
    emitter.push(MachineInstr::Label(1));
    emitter.push(MachineInstr::MovImm {
        dst: Reg::Rax,
        value: 7,
    });
    emitter.push(MachineInstr::Ret);
    assert_eq!(eval::evaluate(&emitter.function, &[]).unwrap(), 7);
}

#[test]
fn resolver_rejects_identity_destination_conflicts_and_temporary_aliases() {
    assert!(
        moves::resolve(
            vec![
                copy(reg(Reg::Rax), reg(Reg::Rax)),
                copy(slot(0), reg(Reg::Rax))
            ],
            reg(Reg::R11),
        )
        .is_err()
    );
    assert!(moves::resolve(vec![copy(reg(Reg::Rax), reg(Reg::Rcx))], reg(Reg::Rax)).is_err());
}
