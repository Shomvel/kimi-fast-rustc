use kfrc_x86::machine::x86::{MachineFunction, MachineInstr, Reg};

fn function(value: i64) -> MachineFunction {
    function_into(Reg::Rcx, value)
}

fn function_into(dst: Reg, value: i64) -> MachineFunction {
    let mut function = MachineFunction::new("immediate");
    function.instructions = vec![MachineInstr::MovImm { dst, value }, MachineInstr::Ret];
    function
}

#[test]
fn unsigned_32_bit_constants_use_zero_extending_mov() {
    let code = function(42).emit_machine_code().unwrap();
    assert_eq!(code, [0xb9, 0x2a, 0x00, 0x00, 0x00, 0xc3]);

    let max = function(i64::from(u32::MAX)).emit_machine_code().unwrap();
    assert_eq!(max, [0xb9, 0xff, 0xff, 0xff, 0xff, 0xc3]);

    let extended = function_into(Reg::R9, 0).emit_machine_code().unwrap();
    assert_eq!(extended, [0x41, 0xb9, 0x00, 0x00, 0x00, 0x00, 0xc3]);
}

#[test]
fn negative_small_constants_use_sign_extended_imm32() {
    let negative = function(-2).emit_machine_code().unwrap();
    assert_eq!(negative, [0x48, 0xc7, 0xc1, 0xfe, 0xff, 0xff, 0xff, 0xc3,]);
}

#[test]
fn large_constants_keep_movabs_and_assembly_matches_the_encoding() {
    let code = function(i64::MAX).emit_machine_code().unwrap();
    assert_eq!(
        code,
        [
            0x48, 0xb9, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x7f, 0xc3,
        ]
    );

    let small = function(42).emit_gnu_assembly().unwrap();
    assert!(small.contains("\tmov rcx, 42\n"));
    assert!(!small.contains("movabs rcx"));
    let large = function(i64::MAX).emit_gnu_assembly().unwrap();
    assert!(large.contains("\tmovabs rcx, 0x7fffffffffffffff\n"));
}
