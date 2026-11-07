use kimi_fast_rustc::x86::{MachineFunction, MachineInstr, Operand, Reg};
use kimi_fast_rustc::x86_encode::EncodeError;
use kimi_fast_rustc::{frontend, machine_eval, native, optimizer_pipeline, placement};

fn compile(source: &str) -> MachineFunction {
    let mut graph = frontend::parse(source).unwrap();
    optimizer_pipeline::optimize(&mut graph);
    let graph = placement::place_for_native(&graph).unwrap();
    native::compile_graph(&graph, "main").unwrap()
}

#[test]
fn arithmetic_uses_immediate_operands_without_constant_moves() {
    let function = compile("fn main(value: i64) -> i64 { return value + 5; }");
    assert!(function.instructions.iter().any(|instruction| matches!(
        instruction,
        MachineInstr::Add {
            src: Operand::Imm(5),
            ..
        }
    )));
    assert!(
        !function
            .instructions
            .iter()
            .any(|instruction| matches!(instruction, MachineInstr::MovImm { .. }))
    );
    assert_eq!(machine_eval::evaluate(&function, &[37]), Ok(42));
}

#[test]
fn bitwise_multiply_and_compare_support_immediates() {
    let function =
        compile("fn main(value: i64) -> i64 { let result = (value & 7) * 3; return result ^ 15; }");
    assert!(function.instructions.iter().any(|instruction| matches!(
        instruction,
        MachineInstr::And {
            src: Operand::Imm(7),
            ..
        }
    )));
    assert!(function.instructions.iter().any(|instruction| matches!(
        instruction,
        MachineInstr::IMul {
            src: Operand::Imm(3),
            ..
        }
    )));
    assert!(function.instructions.iter().any(|instruction| matches!(
        instruction,
        MachineInstr::Xor {
            src: Operand::Imm(15),
            ..
        }
    )));
    assert_eq!(machine_eval::evaluate(&function, &[5]), Ok(0));

    let compare = compile("fn main(value: i64) -> bool { return value == 42; }");
    assert!(compare.instructions.iter().any(|instruction| matches!(
        instruction,
        MachineInstr::Cmp {
            rhs: Operand::Imm(42),
            ..
        }
    )));
    assert_eq!(machine_eval::evaluate(&compare, &[42]), Ok(1));
}

#[test]
fn large_constants_still_use_register_materialization() {
    let function = compile("fn main(value: i64) -> i64 { return value + 2147483648; }");
    assert!(
        function
            .instructions
            .iter()
            .any(|instruction| matches!(instruction, MachineInstr::MovImm { .. }))
    );
    assert_eq!(machine_eval::evaluate(&function, &[0]), Ok(2147483648));
}

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
            0x55, 0x48, 0x89, 0xe5, 0x48, 0x81, 0xc0, 0x05, 0x00, 0x00, 0x00, 0x48, 0x81, 0xe8,
            0x02, 0x00, 0x00, 0x00, 0x48, 0x69, 0xc0, 0x03, 0x00, 0x00, 0x00, 0x48, 0x81, 0xe0,
            0x04, 0x00, 0x00, 0x00, 0x48, 0x81, 0xc8, 0x08, 0x00, 0x00, 0x00, 0x48, 0x81, 0xf0,
            0x10, 0x00, 0x00, 0x00, 0x48, 0x81, 0xf8, 0x07, 0x00, 0x00, 0x00, 0xc9, 0xc3,
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
