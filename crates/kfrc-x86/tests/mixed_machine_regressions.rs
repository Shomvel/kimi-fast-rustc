//! Exact machine encoding and flags regressions retained from mixed old tests.
// Each named module preserves the original source file and test names; only
// source-graph setup was excluded, never the machine assertions.
use kfrc_x86::machine::eval as machine_eval;
use kfrc_x86::machine::x86::{
    Condition, MachineFunction, MachineInstr, Operand, Reg, StackSlot, optimize_zero_idioms,
};
use kfrc_x86::machine::x86_encode::EncodeError;

mod zero_idiom {
    use super::*;
    fn function(instructions: Vec<MachineInstr>) -> MachineFunction {
        let mut function = MachineFunction::new("zero_idiom");
        function.instructions = instructions;
        function
    }
    fn zero(register: Reg) -> MachineInstr {
        MachineInstr::MovImm {
            dst: register,
            value: 0,
        }
    }
    fn self_xor(register: Reg) -> MachineInstr {
        MachineInstr::Xor {
            dst: register,
            src: Operand::Reg(register),
        }
    }
    fn conditions() -> [(Condition, bool); 10] {
        [
            (Condition::Eq, false),
            (Condition::NotEq, true),
            (Condition::SignedLess, true),
            (Condition::SignedLessEq, true),
            (Condition::SignedGreater, false),
            (Condition::SignedGreaterEq, false),
            (Condition::UnsignedBelow, true),
            (Condition::UnsignedBelowEq, true),
            (Condition::UnsignedAbove, false),
            (Condition::UnsignedAboveEq, false),
        ]
    }
    #[test]
    fn exact_bytes_save_four_bytes_per_zero_immediate() {
        let original = function(vec![
            zero(Reg::Rax),
            zero(Reg::R8),
            MachineInstr::MovImm {
                dst: Reg::Rcx,
                value: 1,
            },
            MachineInstr::Ret,
        ]);
        let baseline_size = original.emit_machine_code().unwrap().len();
        let mut optimized = original;
        assert_eq!(optimize_zero_idioms(&mut optimized.instructions), 2);
        assert_eq!(
            optimized.instructions,
            [
                self_xor(Reg::Rax),
                self_xor(Reg::R8),
                MachineInstr::MovImm {
                    dst: Reg::Rcx,
                    value: 1,
                },
                MachineInstr::Ret,
            ]
        );
        let code = optimized.emit_machine_code().unwrap();
        assert_eq!(
            code,
            [
                0x55, 0x48, 0x89, 0xe5, 0x48, 0x31, 0xc0, 0x4d, 0x31, 0xc0, 0x48, 0xc7, 0xc1, 0x01,
                0x00, 0x00, 0x00, 0xc9, 0xc3,
            ]
        );
        assert_eq!(baseline_size - code.len(), 8);
        assert_eq!(machine_eval::evaluate(&optimized, &[]), Ok(0));
    }
    #[test]
    fn cmp_to_setcc_preserves_flags_for_every_condition() {
        for (condition, expected) in conditions() {
            let mut function = function(vec![
                MachineInstr::MovImm {
                    dst: Reg::Rax,
                    value: 4,
                },
                MachineInstr::MovImm {
                    dst: Reg::Rcx,
                    value: 5,
                },
                MachineInstr::Cmp {
                    lhs: Reg::Rax,
                    rhs: Operand::Reg(Reg::Rcx),
                },
                zero(Reg::Rdx),
                MachineInstr::SetCc {
                    condition,
                    dst: Reg::Rax,
                },
                MachineInstr::Ret,
            ]);
            assert_eq!(
                optimize_zero_idioms(&mut function.instructions),
                0,
                "{condition:?}"
            );
            assert_eq!(function.instructions[3], zero(Reg::Rdx));
            assert_eq!(
                machine_eval::evaluate(&function, &[]),
                Ok(i64::from(expected)),
                "{condition:?}"
            );
        }
    }
    #[test]
    fn cmp_to_jcc_preserves_flags_for_every_condition() {
        for (condition, expected) in conditions() {
            let mut function = function(vec![
                MachineInstr::MovImm {
                    dst: Reg::Rax,
                    value: 4,
                },
                MachineInstr::MovImm {
                    dst: Reg::Rcx,
                    value: 5,
                },
                MachineInstr::Cmp {
                    lhs: Reg::Rax,
                    rhs: Operand::Reg(Reg::Rcx),
                },
                zero(Reg::Rdx),
                MachineInstr::Jcc {
                    condition,
                    target: 7,
                },
                zero(Reg::Rax),
                MachineInstr::Ret,
                MachineInstr::Label(7),
                MachineInstr::MovImm {
                    dst: Reg::Rax,
                    value: 1,
                },
                MachineInstr::Ret,
            ]);
            assert_eq!(
                optimize_zero_idioms(&mut function.instructions),
                0,
                "{condition:?}"
            );
            assert_eq!(function.instructions[3], zero(Reg::Rdx));
            assert_eq!(
                machine_eval::evaluate(&function, &[]),
                Ok(i64::from(expected)),
                "{condition:?}"
            );
        }
    }
    #[test]
    fn test_to_jcc_and_dead_flags_allow_the_correct_sites() {
        let mut protected = function(vec![
            MachineInstr::Test { reg: Reg::Rax },
            zero(Reg::Rdx),
            MachineInstr::Jcc {
                condition: Condition::Eq,
                target: 4,
            },
            MachineInstr::Ret,
            MachineInstr::Label(4),
            MachineInstr::Ret,
        ]);
        assert_eq!(optimize_zero_idioms(&mut protected.instructions), 0);
        assert_eq!(protected.instructions[1], zero(Reg::Rdx));

        let mut setcc = function(vec![
            MachineInstr::Test { reg: Reg::Rax },
            zero(Reg::Rdx),
            MachineInstr::SetCc {
                condition: Condition::Eq,
                dst: Reg::Rax,
            },
            MachineInstr::Ret,
        ]);
        assert_eq!(optimize_zero_idioms(&mut setcc.instructions), 0);
        assert_eq!(setcc.instructions[1], zero(Reg::Rdx));
        assert_eq!(machine_eval::evaluate(&setcc, &[]), Ok(1));

        let mut dead = function(vec![
            zero(Reg::Rdx),
            MachineInstr::Cmp {
                lhs: Reg::Rax,
                rhs: Operand::Reg(Reg::Rcx),
            },
            MachineInstr::SetCc {
                condition: Condition::Eq,
                dst: Reg::Rax,
            },
            zero(Reg::Rdx),
            MachineInstr::Ret,
        ]);
        assert_eq!(optimize_zero_idioms(&mut dead.instructions), 2);
        assert_eq!(dead.instructions[0], self_xor(Reg::Rdx));
        assert_eq!(dead.instructions[3], self_xor(Reg::Rdx));
        assert_eq!(machine_eval::evaluate(&dead, &[]), Ok(1));
    }
    #[test]
    fn call_multiple_return_and_loop_label_boundaries_are_conservative() {
        let mut call = function(vec![
            zero(Reg::Rdx),
            MachineInstr::Call { target: 0 },
            MachineInstr::Ret,
        ]);
        call.symbols.push("target".into());
        assert_eq!(optimize_zero_idioms(&mut call.instructions), 0);
        assert_eq!(call.instructions[0], zero(Reg::Rdx));

        let mut returns = function(vec![
            zero(Reg::Rdx),
            MachineInstr::Ret,
            zero(Reg::Rcx),
            MachineInstr::Ret,
        ]);
        assert_eq!(optimize_zero_idioms(&mut returns.instructions), 0);
        assert_eq!(returns.instructions[0], zero(Reg::Rdx));
        assert_eq!(returns.instructions[2], zero(Reg::Rcx));

        let mut loop_header = function(vec![
            MachineInstr::Label(9),
            zero(Reg::Rdx),
            MachineInstr::Jmp(9),
            MachineInstr::Ret,
        ]);
        assert_eq!(optimize_zero_idioms(&mut loop_header.instructions), 0);
        assert_eq!(loop_header.instructions[1], zero(Reg::Rdx));

        let mut conditional_back_edge = function(vec![
            MachineInstr::Label(9),
            zero(Reg::Rdx),
            MachineInstr::Jcc {
                condition: Condition::NotEq,
                target: 9,
            },
            MachineInstr::Ret,
        ]);
        assert_eq!(
            optimize_zero_idioms(&mut conditional_back_edge.instructions),
            0
        );
        assert_eq!(conditional_back_edge.instructions[1], zero(Reg::Rdx));
    }
}

mod zero_compare {
    use super::*;
    const VALUES: [i64; 5] = [i64::MIN, -1, 0, 1, i64::MAX];
    fn conditions() -> [Condition; 10] {
        [
            Condition::Eq,
            Condition::NotEq,
            Condition::SignedLess,
            Condition::SignedLessEq,
            Condition::SignedGreater,
            Condition::SignedGreaterEq,
            Condition::UnsignedBelow,
            Condition::UnsignedBelowEq,
            Condition::UnsignedAbove,
            Condition::UnsignedAboveEq,
        ]
    }
    fn function(instructions: Vec<MachineInstr>) -> MachineFunction {
        let mut function = MachineFunction::new("zero_compare");
        function.instructions = instructions;
        function
    }
    fn zero_condition(condition: Condition, value: i64) -> i64 {
        i64::from(match condition {
            Condition::Eq => value == 0,
            Condition::NotEq => value != 0,
            Condition::SignedLess => value < 0,
            Condition::SignedLessEq => value <= 0,
            Condition::SignedGreater => value > 0,
            Condition::SignedGreaterEq => value >= 0,
            Condition::UnsignedBelow => false,
            Condition::UnsignedBelowEq => value == 0,
            Condition::UnsignedAbove => value != 0,
            Condition::UnsignedAboveEq => true,
        })
    }
    #[test]
    fn test_and_cmp_zero_have_exact_register_encodings() {
        for (reg, rex, modrm) in [(Reg::Rax, 0x48, 0xc0), (Reg::R11, 0x4d, 0xdb)] {
            let expected = vec![0x55, 0x48, 0x89, 0xe5, rex, 0x85, modrm, 0xc9, 0xc3];
            for instruction in [
                MachineInstr::Test { reg },
                MachineInstr::Cmp {
                    lhs: reg,
                    rhs: Operand::Imm(0),
                },
            ] {
                let function = function(vec![instruction, MachineInstr::Ret]);
                assert_eq!(
                    function.emit_machine_code().unwrap(),
                    expected,
                    "{instruction:?}"
                );
                let assembly = function.emit_gnu_assembly().unwrap();
                assert!(
                    assembly.contains(&format!("\ttest {reg}, {reg}\n")),
                    "{assembly}"
                );
                assert!(!assembly.contains("\tcmp "), "{assembly}");
            }
        }
    }
    #[test]
    fn test_zero_preserves_every_setcc_condition_and_opcode() {
        for condition in conditions() {
            let opcode = match condition {
                Condition::Eq => 0x94,
                Condition::NotEq => 0x95,
                Condition::SignedLess => 0x9c,
                Condition::SignedLessEq => 0x9e,
                Condition::SignedGreater => 0x9f,
                Condition::SignedGreaterEq => 0x9d,
                Condition::UnsignedBelow => 0x92,
                Condition::UnsignedBelowEq => 0x96,
                Condition::UnsignedAbove => 0x97,
                Condition::UnsignedAboveEq => 0x93,
            };
            let function = function(vec![
                MachineInstr::LoadArg {
                    dst: Reg::Rax,
                    index: 0,
                },
                MachineInstr::Test { reg: Reg::Rax },
                MachineInstr::SetCc {
                    condition,
                    dst: Reg::Rax,
                },
                MachineInstr::Ret,
            ]);
            assert_eq!(
                function.emit_machine_code().unwrap(),
                [
                    0x55, 0x48, 0x89, 0xe5, 0x48, 0x89, 0xf8, 0x48, 0x85, 0xc0, 0x0f, opcode, 0xc0,
                    0x0f, 0xb6, 0xc0, 0xc9, 0xc3,
                ],
                "{condition:?}"
            );
            for value in VALUES {
                assert_eq!(
                    machine_eval::evaluate(&function, &[value]),
                    Ok(zero_condition(condition, value)),
                    "{condition:?}, value {value}"
                );
            }
        }
    }
    #[test]
    fn spilled_cmp_rhs_keeps_cmp_encoding_and_semantics() {
        let mut function = function(vec![
            MachineInstr::MovImm {
                dst: Reg::Rcx,
                value: -1,
            },
            MachineInstr::Store {
                slot: StackSlot(0),
                src: Reg::Rcx,
            },
            MachineInstr::MovImm {
                dst: Reg::Rax,
                value: 0,
            },
            MachineInstr::Cmp {
                lhs: Reg::Rax,
                rhs: Operand::Stack(StackSlot(0)),
            },
            MachineInstr::SetCc {
                condition: Condition::SignedGreater,
                dst: Reg::Rax,
            },
            MachineInstr::Ret,
        ]);
        function.stack_slots = 1;
        let code = function.emit_machine_code().unwrap();
        assert!(
            code.windows(4)
                .any(|bytes| bytes == [0x48, 0x3b, 0x45, 0xf8])
        );
        assert!(
            !function
                .instructions
                .iter()
                .any(|instruction| matches!(instruction, MachineInstr::Test { .. }))
        );
        let assembly = function.emit_gnu_assembly().unwrap();
        assert!(
            assembly.contains("\tcmp rax, QWORD PTR [rbp-8]\n"),
            "{assembly}"
        );
        assert_eq!(machine_eval::evaluate(&function, &[]), Ok(1));
    }
}

mod immediate_arithmetic {
    use super::*;
    #[test]
    fn immediate_group_and_multiply_encodings_are_exact() {
        let mut function = MachineFunction::new("immediate_arithmetic");
        function.instructions = vec![
            MachineInstr::Add {
                dst: Reg::Rax,
                src: Operand::Imm(5),
            },
            MachineInstr::Sub {
                dst: Reg::Rax,
                src: Operand::Imm(2),
            },
            MachineInstr::IMul {
                dst: Reg::Rax,
                src: Operand::Imm(3),
            },
            MachineInstr::And {
                dst: Reg::Rax,
                src: Operand::Imm(4),
            },
            MachineInstr::Or {
                dst: Reg::Rax,
                src: Operand::Imm(8),
            },
            MachineInstr::Xor {
                dst: Reg::Rax,
                src: Operand::Imm(16),
            },
            MachineInstr::Cmp {
                lhs: Reg::Rax,
                rhs: Operand::Imm(7),
            },
            MachineInstr::Ret,
        ];
        assert_eq!(
            function.emit_machine_code().unwrap(),
            [
                0x55, 0x48, 0x89, 0xe5, 0x48, 0x83, 0xc0, 0x05, 0x48, 0x83, 0xe8, 0x02, 0x48, 0x6b,
                0xc0, 0x03, 0x48, 0x83, 0xe0, 0x04, 0x48, 0x83, 0xc8, 0x08, 0x48, 0x83, 0xf0, 0x10,
                0x48, 0x83, 0xf8, 0x07, 0xc9, 0xc3,
            ]
        );
    }
    #[test]
    fn immediate_division_is_rejected_explicitly() {
        let mut function = MachineFunction::new("bad_division");
        function.instructions = vec![
            MachineInstr::IDiv {
                src: Operand::Imm(2),
            },
            MachineInstr::Ret,
        ];
        assert_eq!(
            function.emit_machine_code(),
            Err(EncodeError::ImmediateDivision)
        );
    }
}

mod immediate_shifts {
    use super::*;
    #[test]
    fn immediate_shift_machine_encodings_are_exact() {
        let mut function = MachineFunction::new("immediate_shifts");
        function.instructions = vec![
            MachineInstr::ShlImm {
                dst: Reg::Rax,
                count: 3,
            },
            MachineInstr::ShrImm {
                dst: Reg::Rax,
                count: 4,
            },
            MachineInstr::LShrImm {
                dst: Reg::Rax,
                count: 5,
            },
            MachineInstr::Ret,
        ];
        assert_eq!(
            function.emit_machine_code().unwrap(),
            [
                0x55, 0x48, 0x89, 0xe5, 0x48, 0xc1, 0xe0, 0x03, 0x48, 0xc1, 0xf8, 0x04, 0x48, 0xc1,
                0xe8, 0x05, 0xc9, 0xc3,
            ]
        );
    }
}

mod loop_control {
    use super::*;

    #[test]
    fn evaluator_stops_an_actual_infinite_loop() {
        let mut function = MachineFunction::new("infinite_loop");
        function.instructions = vec![
            MachineInstr::Label(0),
            MachineInstr::Jmp(0),
            MachineInstr::Ret,
        ];
        assert_eq!(
            machine_eval::evaluate(&function, &[]),
            Err(machine_eval::MachineEvalError::InstructionLimitExceeded)
        );
    }
}
