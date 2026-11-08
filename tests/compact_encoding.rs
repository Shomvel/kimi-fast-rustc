use kimi_fast_rustc::x86::{Condition, MachineFunction, MachineInstr, Operand, Reg, StackSlot};
use kimi_fast_rustc::x86_encode::encode_with_relocations;

fn encode_instruction(instruction: MachineInstr) -> Vec<u8> {
    let mut function = MachineFunction::new("compact_instruction");
    function.instructions = vec![instruction, MachineInstr::Ret];
    let code = function.emit_machine_code().unwrap();
    code[4..code.len() - 2].to_vec()
}

#[test]
fn group_one_immediates_use_imm8_only_in_the_signed_byte_range() {
    for reg in [Reg::Rax, Reg::R8, Reg::R15] {
        let low_code = match reg {
            Reg::Rax | Reg::R8 => 0,
            Reg::R15 => 7,
            _ => unreachable!(),
        };
        let rex = if reg == Reg::Rax { 0x48 } else { 0x49 };
        for value in [-129_i32, -128, 127, 128] {
            let instructions = [
                (
                    MachineInstr::Add {
                        dst: reg,
                        src: Operand::Imm(value),
                    },
                    0,
                ),
                (
                    MachineInstr::Or {
                        dst: reg,
                        src: Operand::Imm(value),
                    },
                    1,
                ),
                (
                    MachineInstr::And {
                        dst: reg,
                        src: Operand::Imm(value),
                    },
                    4,
                ),
                (
                    MachineInstr::Sub {
                        dst: reg,
                        src: Operand::Imm(value),
                    },
                    5,
                ),
                (
                    MachineInstr::Xor {
                        dst: reg,
                        src: Operand::Imm(value),
                    },
                    6,
                ),
                (
                    MachineInstr::Cmp {
                        lhs: reg,
                        rhs: Operand::Imm(value),
                    },
                    7,
                ),
            ];
            for (instruction, extension) in instructions {
                let compact = i8::try_from(value).is_ok();
                let mut expected = vec![
                    rex,
                    if compact { 0x83 } else { 0x81 },
                    0xc0 | (extension << 3) | low_code,
                ];
                if compact {
                    expected.push(value as u8);
                } else {
                    expected.extend_from_slice(&value.to_le_bytes());
                }
                assert_eq!(encode_instruction(instruction), expected, "{instruction:?}");
            }
        }
    }
}

#[test]
fn immediate_multiply_uses_imm8_and_extended_register_rex_bits() {
    for reg in [Reg::Rax, Reg::R8, Reg::R15] {
        let low_code = match reg {
            Reg::Rax | Reg::R8 => 0,
            Reg::R15 => 7,
            _ => unreachable!(),
        };
        let rex = if reg == Reg::Rax { 0x48 } else { 0x4d };
        for value in [-129_i32, -128, 127, 128] {
            let compact = i8::try_from(value).is_ok();
            let mut expected = vec![
                rex,
                if compact { 0x6b } else { 0x69 },
                0xc0 | (low_code << 3) | low_code,
            ];
            if compact {
                expected.push(value as u8);
            } else {
                expected.extend_from_slice(&value.to_le_bytes());
            }
            assert_eq!(
                encode_instruction(MachineInstr::IMul {
                    dst: reg,
                    src: Operand::Imm(value),
                }),
                expected,
                "imul {reg:?}, {value}"
            );
        }
    }
}

#[test]
fn prologue_subtraction_uses_imm8_only_through_a_112_byte_frame() {
    let cases: &[(u32, &[u8])] = &[
        (0, &[0x55, 0x48, 0x89, 0xe5, 0xc9, 0xc3]),
        (
            1,
            &[0x55, 0x48, 0x89, 0xe5, 0x48, 0x83, 0xec, 0x10, 0xc9, 0xc3],
        ),
        (
            14,
            &[0x55, 0x48, 0x89, 0xe5, 0x48, 0x83, 0xec, 0x70, 0xc9, 0xc3],
        ),
        (
            16,
            &[
                0x55, 0x48, 0x89, 0xe5, 0x48, 0x81, 0xec, 0x80, 0x00, 0x00, 0x00, 0xc9, 0xc3,
            ],
        ),
        (
            17,
            &[
                0x55, 0x48, 0x89, 0xe5, 0x48, 0x81, 0xec, 0x90, 0x00, 0x00, 0x00, 0xc9, 0xc3,
            ],
        ),
    ];
    for (stack_slots, expected) in cases {
        let mut function = MachineFunction::new("compact_prologue");
        function.stack_slots = *stack_slots;
        function.instructions = vec![MachineInstr::Ret];
        assert_eq!(function.emit_machine_code().unwrap(), *expected);
    }
}

#[test]
fn rbp_memory_uses_disp8_at_both_boundaries_and_keeps_rex() {
    let mut spills = MachineFunction::new("compact_spills");
    spills.stack_slots = 17;
    spills.instructions = vec![
        MachineInstr::Load {
            dst: Reg::Rax,
            slot: StackSlot(15),
        },
        MachineInstr::Load {
            dst: Reg::R8,
            slot: StackSlot(15),
        },
        MachineInstr::Load {
            dst: Reg::R15,
            slot: StackSlot(15),
        },
        MachineInstr::Load {
            dst: Reg::R15,
            slot: StackSlot(16),
        },
        MachineInstr::Ret,
    ];
    assert_eq!(
        spills.emit_machine_code().unwrap(),
        [
            0x55, 0x48, 0x89, 0xe5, 0x48, 0x81, 0xec, 0x90, 0x00, 0x00, 0x00, 0x48, 0x8b, 0x45,
            0x80, 0x4c, 0x8b, 0x45, 0x80, 0x4c, 0x8b, 0x7d, 0x80, 0x4c, 0x8b, 0xbd, 0x78, 0xff,
            0xff, 0xff, 0xc9, 0xc3,
        ]
    );

    let mut incoming = MachineFunction::new("compact_incoming");
    incoming.instructions = vec![
        MachineInstr::LoadArg {
            dst: Reg::Rax,
            index: 19,
        },
        MachineInstr::LoadArg {
            dst: Reg::R8,
            index: 19,
        },
        MachineInstr::LoadArg {
            dst: Reg::R15,
            index: 19,
        },
        MachineInstr::LoadArg {
            dst: Reg::R15,
            index: 20,
        },
        MachineInstr::LoadArg {
            dst: Reg::R15,
            index: 21,
        },
        MachineInstr::Ret,
    ];
    assert_eq!(
        incoming.emit_machine_code().unwrap(),
        [
            0x55, 0x48, 0x89, 0xe5, 0x48, 0x8b, 0x45, 0x78, 0x4c, 0x8b, 0x45, 0x78, 0x4c, 0x8b,
            0x7d, 0x78, 0x4c, 0x8b, 0xbd, 0x80, 0x00, 0x00, 0x00, 0x4c, 0x8b, 0xbd, 0x88, 0x00,
            0x00, 0x00, 0xc9, 0xc3,
        ]
    );
}

#[test]
fn rsp_memory_omits_zero_displacement_and_keeps_sib_and_rex() {
    let mut function = MachineFunction::new("compact_outgoing");
    function.instructions = vec![
        MachineInstr::StoreArg {
            src: Reg::Rax,
            index: 6,
        },
        MachineInstr::StoreArg {
            src: Reg::R8,
            index: 21,
        },
        MachineInstr::StoreArg {
            src: Reg::R15,
            index: 21,
        },
        MachineInstr::StoreArg {
            src: Reg::R15,
            index: 22,
        },
        MachineInstr::StoreArg {
            src: Reg::R15,
            index: 23,
        },
        MachineInstr::Ret,
    ];
    assert_eq!(
        function.emit_machine_code().unwrap(),
        [
            0x55, 0x48, 0x89, 0xe5, 0x48, 0x89, 0x04, 0x24, 0x4c, 0x89, 0x44, 0x24, 0x78, 0x4c,
            0x89, 0x7c, 0x24, 0x78, 0x4c, 0x89, 0xbc, 0x24, 0x80, 0x00, 0x00, 0x00, 0x4c, 0x89,
            0xbc, 0x24, 0x88, 0x00, 0x00, 0x00, 0xc9, 0xc3,
        ]
    );
}

#[test]
fn compact_instructions_preserve_call_relocations_and_local_jump_fixups() {
    let mut function = MachineFunction::new("compact_control_flow");
    function.symbols = vec!["callee".into()];
    function.instructions = vec![
        MachineInstr::Add {
            dst: Reg::Rax,
            src: Operand::Imm(127),
        },
        MachineInstr::Call { target: 0 },
        MachineInstr::Jmp(1),
        MachineInstr::Sub {
            dst: Reg::R8,
            src: Operand::Imm(-128),
        },
        MachineInstr::Label(1),
        MachineInstr::Jcc {
            condition: Condition::Eq,
            target: 2,
        },
        MachineInstr::IMul {
            dst: Reg::R15,
            src: Operand::Imm(128),
        },
        MachineInstr::StoreArg {
            src: Reg::R15,
            index: 6,
        },
        MachineInstr::Label(2),
        MachineInstr::Ret,
    ];
    let (code, relocations) = encode_with_relocations(&function).unwrap();
    assert_eq!(
        code,
        [
            0x55, 0x48, 0x89, 0xe5, 0x48, 0x83, 0xc0, 0x7f, 0xe8, 0x00, 0x00, 0x00, 0x00, 0xeb,
            0x04, 0x49, 0x83, 0xe8, 0x80, 0x74, 0x0b, 0x4d, 0x69, 0xff, 0x80, 0x00, 0x00, 0x00,
            0x4c, 0x89, 0x3c, 0x24, 0xc9, 0xc3,
        ]
    );
    assert_eq!(relocations.len(), 1);
    assert_eq!(relocations[0].offset, 9);
    assert_eq!(relocations[0].target, "callee");
}
