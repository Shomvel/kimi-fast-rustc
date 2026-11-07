use kimi_fast_rustc::x86::{MachineFunction, MachineInstr, Reg};

#[test]
fn outgoing_stack_arguments_include_rsp_sib_byte() {
    let mut function = MachineFunction::new("stack_args");
    function.instructions = vec![
        MachineInstr::StoreArg {
            src: Reg::R11,
            index: 6,
        },
        MachineInstr::StoreArg {
            src: Reg::R11,
            index: 7,
        },
        MachineInstr::Ret,
    ];
    let code = function.emit_machine_code().unwrap();
    assert_eq!(
        code,
        [
            0x55, 0x48, 0x89, 0xe5, 0x4c, 0x89, 0x9c, 0x24, 0x00, 0x00, 0x00, 0x00, 0x4c, 0x89,
            0x9c, 0x24, 0x08, 0x00, 0x00, 0x00, 0xc9, 0xc3,
        ]
    );
}
