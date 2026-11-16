use super::*;
use crate::machine::eval;

fn function(code: Vec<MachineInstr>) -> MachineFunction {
    let mut function = MachineFunction::new("rematerialize");
    function.stack_slots = 8;
    function.instructions = code;
    function
}

fn constant(value: i64) -> Vec<MachineInstr> {
    vec![
        MachineInstr::MovImm {
            dst: Reg::Rax,
            value,
        },
        MachineInstr::Store {
            slot: StackSlot(0),
            src: Reg::Rax,
        },
        MachineInstr::Load {
            dst: Reg::Rax,
            slot: StackSlot(0),
        },
        MachineInstr::Ret,
    ]
}

#[test]
fn constant_spills_recreate_without_changing_live_flags() {
    for value in [i32::MIN as i64, -1, 0, 1, i32::MAX as i64] {
        let mut code = constant(value);
        code.insert(
            2,
            MachineInstr::Cmp {
                lhs: Reg::Rdi,
                rhs: MachineOperand::Imm(7),
            },
        );
        code.splice(
            4..5,
            [
                MachineInstr::Jcc {
                    condition: Condition::Eq,
                    target: 1,
                },
                MachineInstr::MovImm {
                    dst: Reg::Rax,
                    value: 123,
                },
                MachineInstr::Ret,
                MachineInstr::Label(1),
                MachineInstr::Ret,
            ],
        );
        let original = function(code);
        let mut optimized = original.clone();
        let stats = run(&mut optimized, 1);
        assert_eq!(
            (stats.slots, stats.loads_recreated, stats.stores_removed),
            (1, 1, 1)
        );
        for argument in [i64::MIN, -1, 0, 6, 7, 8, i64::MAX] {
            assert_eq!(
                eval::evaluate(&original, &[argument]),
                eval::evaluate(&optimized, &[argument])
            );
        }
        optimized.emit_machine_code().unwrap();
    }
}

fn diamond(a: Option<i64>, b: Option<i64>) -> MachineFunction {
    let mut code = vec![
        MachineInstr::Test { reg: Reg::Rdi },
        MachineInstr::Jcc {
            condition: Condition::Eq,
            target: 1,
        },
    ];
    if let Some(value) = a {
        code.extend(constant(value)[..2].iter().copied());
    }
    code.extend([MachineInstr::Jmp(2), MachineInstr::Label(1)]);
    if let Some(value) = b {
        code.extend(constant(value)[..2].iter().copied());
    }
    code.extend([
        MachineInstr::Label(2),
        MachineInstr::Load {
            dst: Reg::Rax,
            slot: StackSlot(0),
        },
        MachineInstr::Ret,
    ]);
    function(code)
}

#[test]
fn joins_require_one_recipe_and_initialization_on_every_path() {
    for (a, b, expected) in [
        (Some(19), Some(19), 1),
        (Some(19), Some(20), 0),
        (Some(19), None, 0),
        (None, Some(19), 0),
    ] {
        let original = diamond(a, b);
        let mut optimized = original.clone();
        assert_eq!(run(&mut optimized, 1).slots, expected);
        for argument in [-1, 0, 1] {
            assert_eq!(
                eval::evaluate(&original, &[argument]),
                eval::evaluate(&optimized, &[argument])
            );
        }
    }
}

#[test]
fn loop_backedges_do_not_prove_the_first_iteration_initialized() {
    for entry_store in [false, true] {
        let mut code = Vec::new();
        if entry_store {
            code.extend(constant(17)[..2].iter().copied());
        }
        code.extend([
            MachineInstr::Mov {
                dst: Reg::Rcx,
                src: Reg::Rdi,
            },
            MachineInstr::Label(1),
            MachineInstr::Load {
                dst: Reg::Rax,
                slot: StackSlot(0),
            },
            MachineInstr::Test { reg: Reg::Rcx },
            MachineInstr::Jcc {
                condition: Condition::Eq,
                target: 2,
            },
            MachineInstr::MovImm {
                dst: Reg::Rax,
                value: 17,
            },
            MachineInstr::Store {
                slot: StackSlot(0),
                src: Reg::Rax,
            },
            MachineInstr::Sub {
                dst: Reg::Rcx,
                src: MachineOperand::Imm(1),
            },
            MachineInstr::Jmp(1),
            MachineInstr::Label(2),
            MachineInstr::Ret,
        ]);
        let original = function(code);
        let mut optimized = original.clone();
        assert_eq!(run(&mut optimized, 1).slots, usize::from(entry_store));
        for count in 0..64 {
            assert_eq!(
                eval::evaluate(&original, &[count]),
                eval::evaluate(&optimized, &[count])
            );
        }
    }
}

#[test]
fn reused_slots_and_stale_register_recipes_are_retained() {
    for clobber in [
        MachineInstr::LoadArg {
            dst: Reg::Rax,
            index: 0,
        },
        MachineInstr::Neg { dst: Reg::Rax },
        MachineInstr::Cqo,
        MachineInstr::MulHigh { src: Reg::Rsi },
        MachineInstr::Call { target: 0 },
        MachineInstr::Label(1),
    ] {
        let mut code = constant(17);
        code.insert(1, clobber);
        let original = function(code);
        let mut optimized = original.clone();
        assert_eq!(run(&mut optimized, 1).slots, 0, "{clobber:?}");
        assert_eq!(optimized, original);
    }
    let mut code = constant(17);
    code.splice(
        2..2,
        [
            MachineInstr::LoadArg {
                dst: Reg::Rax,
                index: 0,
            },
            MachineInstr::Store {
                slot: StackSlot(0),
                src: Reg::Rax,
            },
        ],
    );
    let original = function(code);
    let mut optimized = original.clone();
    assert_eq!(run(&mut optimized, 1).slots, 0);
    assert_eq!(optimized, original);
}

#[test]
fn source_storage_and_memory_operands_are_not_spill_recipes() {
    for spill_count in [0, 1] {
        let mut code = constant(17);
        code.insert(
            2,
            MachineInstr::Add {
                dst: Reg::Rdi,
                src: MachineOperand::Stack(StackSlot(0)),
            },
        );
        let original = function(code);
        let mut optimized = original.clone();
        assert_eq!(run(&mut optimized, spill_count).slots, 0);
        assert_eq!(optimized, original);
    }
    let original = function(constant(i64::MAX));
    let mut optimized = original.clone();
    assert_eq!(run(&mut optimized, 1).slots, 0);
    assert_eq!(optimized, original);
}

#[test]
fn independent_address_recipes_keep_displacements_and_relocations() {
    use crate::machine::x86_encode::{AddressRelocationKind, encode_with_all_relocations};

    for recipe in [
        Recipe::Frame(-32),
        Recipe::Frame(-64),
        Recipe::Symbol(0, -7),
        Recipe::Symbol(1, 21),
    ] {
        let mut code = constant(0);
        code[0] = recipe.instruction(Reg::Rax);
        let mut optimized = function(code);
        optimized.symbols.push("rematerialize_object".into());
        optimized.symbols.push("other_rematerialize_object".into());
        let stats = run(&mut optimized, 1);
        assert_eq!(
            (stats.slots, stats.loads_recreated, stats.stores_removed),
            (1, 1, 1)
        );
        assert_eq!(optimized.instructions[1], recipe.instruction(Reg::Rax));
        let (_, calls, addresses) = encode_with_all_relocations(&optimized).unwrap();
        assert!(calls.is_empty());
        match recipe {
            Recipe::Symbol(symbol, addend) => {
                assert_eq!(addresses.len(), 2);
                for address in addresses {
                    assert_eq!(address.target, optimized.symbols[symbol as usize]);
                    assert_eq!(address.addend, addend);
                    assert_eq!(address.kind, AddressRelocationKind::PcRelative);
                }
            }
            Recipe::Frame(_) => assert!(addresses.is_empty()),
            Recipe::Immediate(_) => unreachable!(),
        }
    }
    // Aligned StackAddress emission includes AND; its result is not a pure
    // frame-displacement recipe. TLS may depend on the execution thread.
    for producer in [
        MachineInstr::ThreadLocalAddress {
            dst: Reg::Rax,
            symbol: 0,
        },
        MachineInstr::StackAddress {
            dst: Reg::Rax,
            offset: -8,
        },
    ] {
        let mut code = constant(0);
        code[0] = producer;
        let original = function(code);
        let mut optimized = original.clone();
        assert_eq!(run(&mut optimized, 1).slots, 0);
        assert_eq!(optimized, original);
    }
    let mut code = constant(0);
    code.splice(
        0..1,
        [
            MachineInstr::StackAddress {
                dst: Reg::Rax,
                offset: -32,
            },
            MachineInstr::And {
                dst: Reg::Rax,
                src: MachineOperand::Imm(-32),
            },
        ],
    );
    let original = function(code);
    let mut optimized = original.clone();
    assert_eq!(run(&mut optimized, 1).slots, 0);
    assert_eq!(optimized, original);
}

#[test]
fn budget_exhaustion_never_publishes_partial_spill_removal() {
    let original = diamond(Some(19), Some(19));
    let mut bailouts = 0;
    for budget in 0..80 {
        let mut optimized = original.clone();
        let stats = run_with_budget(&mut optimized, 1, budget);
        assert!(stats.visits <= budget);
        if stats.budget_bailouts != 0 {
            bailouts += 1;
            assert_eq!(optimized, original);
        } else {
            assert_eq!(stats.slots, 1);
        }
    }
    assert!(bailouts > 0 && bailouts < 80);
}

#[test]
fn candidate_limit_keeps_omitted_slots_intact() {
    let mut code = Vec::new();
    for slot in 0..65 {
        code.extend([
            MachineInstr::MovImm {
                dst: Reg::Rax,
                value: slot as i64,
            },
            MachineInstr::Store {
                slot: StackSlot(slot),
                src: Reg::Rax,
            },
            MachineInstr::Load {
                dst: Reg::Rax,
                slot: StackSlot(slot),
            },
        ]);
    }
    code.push(MachineInstr::Ret);
    let mut original = function(code);
    original.stack_slots = 65;
    let mut optimized = original.clone();
    let stats = run(&mut optimized, 65);
    assert_eq!(
        (stats.slots, stats.loads_recreated, stats.stores_removed),
        (64, 64, 64)
    );
    assert_eq!(
        eval::evaluate(&original, &[]),
        eval::evaluate(&optimized, &[])
    );
    assert!(optimized.instructions.iter().any(|i| matches!(
        i,
        MachineInstr::Store {
            slot: StackSlot(64),
            ..
        }
    )));
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
mod native;
