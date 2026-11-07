use kimi_fast_rustc::x86::{MachineFunction, MachineInstr, Reg};

fn function(value: i64) -> MachineFunction {
    let mut function = MachineFunction::new("immediate");
    function.instructions = vec![
        MachineInstr::MovImm {
            dst: Reg::Rcx,
            value,
        },
        MachineInstr::Ret,
    ];
    function
}

#[test]
fn small_constants_use_sign_extended_imm32() {
    let code = function(42).emit_machine_code().unwrap();
    assert_eq!(
        code,
        [
            0x55, 0x48, 0x89, 0xe5, 0x48, 0xc7, 0xc1, 0x2a, 0x00, 0x00, 0x00, 0xc9, 0xc3,
        ]
    );

    let negative = function(-2).emit_machine_code().unwrap();
    assert_eq!(
        negative,
        [
            0x55, 0x48, 0x89, 0xe5, 0x48, 0xc7, 0xc1, 0xfe, 0xff, 0xff, 0xff, 0xc9, 0xc3,
        ]
    );
}

#[test]
fn large_constants_keep_movabs_and_assembly_matches_the_encoding() {
    let code = function(i64::MAX).emit_machine_code().unwrap();
    assert_eq!(
        code,
        [
            0x55, 0x48, 0x89, 0xe5, 0x48, 0xb9, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x7f,
            0xc9, 0xc3,
        ]
    );

    let small = function(42).emit_gnu_assembly().unwrap();
    assert!(small.contains("\tmov rcx, 42\n"));
    assert!(!small.contains("movabs rcx"));
    let large = function(i64::MAX).emit_gnu_assembly().unwrap();
    assert!(large.contains("\tmovabs rcx, 0x7fffffffffffffff\n"));
}
