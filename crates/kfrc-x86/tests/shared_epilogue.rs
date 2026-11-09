//! Returns share one callee-saved restore sequence.
use kfrc_x86::machine::x86::{Condition, MachineFunction, MachineInstr, Reg};

fn function(callee_saved: Vec<Reg>) -> MachineFunction {
    let mut function = MachineFunction::new("shared_epilogue");
    function.callee_saved = callee_saved;
    function.instructions = vec![
        MachineInstr::Test { reg: Reg::Rdi },
        MachineInstr::Jcc {
            condition: Condition::Eq,
            target: 1,
        },
        MachineInstr::MovImm {
            dst: Reg::Rax,
            value: 7,
        },
        MachineInstr::Ret,
        MachineInstr::Label(1),
        MachineInstr::MovImm {
            dst: Reg::Rax,
            value: 9,
        },
        MachineInstr::Ret,
    ];
    function
}

fn count(code: &[u8], needle: &[u8]) -> usize {
    code.windows(needle.len()).filter(|w| *w == needle).count()
}

#[test]
fn later_returns_jump_to_the_first_epilogue() {
    let code = function(vec![Reg::Rbx]).emit_machine_code().unwrap();
    // One `lea rsp, [rbp - 8]; pop rbx; leave; ret`, reached by a short backward jump.
    assert_eq!(count(&code, &[0x48, 0x8d, 0x65, 0xf8, 0x5b, 0xc9, 0xc3]), 1, "{code:x?}");
    assert_eq!(code.iter().filter(|&&b| b == 0xc3).count(), 1, "{code:x?}");
    assert_eq!(code[code.len() - 2], 0xeb, "{code:x?}");
}

#[test]
fn returns_without_callee_saved_registers_stay_inline() {
    let code = function(Vec::new()).emit_machine_code().unwrap();
    assert_eq!(code.iter().filter(|&&b| b == 0xc3).count(), 2, "{code:x?}");
}
