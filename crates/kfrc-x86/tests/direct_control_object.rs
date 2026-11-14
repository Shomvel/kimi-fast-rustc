use kfrc_x86::machine::x86::{Condition, MachineFunction, MachineInstr};
use kfrc_x86::machine::x86_encode::EncodeError;

fn function(instructions: Vec<MachineInstr>) -> MachineFunction {
    let mut function = MachineFunction::new("control");
    function.instructions = instructions;
    function
}

#[test]
fn backward_jump_uses_next_instruction_relative_displacement() {
    let code = function(vec![
        MachineInstr::Label(1),
        MachineInstr::Jmp(1),
        MachineInstr::Ret,
    ])
    .emit_machine_code()
    .unwrap();
    assert_eq!(code, [0xeb, 0xfe, 0xc3]);
}

#[test]
fn conditional_jump_uses_short_condition_encoding() {
    let code = function(vec![
        MachineInstr::Label(1),
        MachineInstr::Jcc {
            condition: Condition::Eq,
            target: 1,
        },
        MachineInstr::Ret,
    ])
    .emit_machine_code()
    .unwrap();
    assert_eq!(code, [0x74, 0xfe, 0xc3]);
}

#[test]
fn unknown_and_duplicate_labels_are_rejected() {
    let error = function(vec![MachineInstr::Jmp(99), MachineInstr::Ret])
        .emit_machine_code()
        .unwrap_err();
    assert_eq!(error, EncodeError::UnknownLabel(99));

    let error = function(vec![
        MachineInstr::Label(1),
        MachineInstr::Label(1),
        MachineInstr::Ret,
    ])
    .emit_machine_code()
    .unwrap_err();
    assert_eq!(error, EncodeError::DuplicateLabel(1));
}
