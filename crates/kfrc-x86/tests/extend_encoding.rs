//! Width canonicalization uses movzx/movsx/movsxd and zero-extending 32-bit moves.
use kfrc_x86::machine::eval;
use kfrc_x86::machine::x86::{BitWidth, MachineFunction, MachineInstr, Reg};

fn extend(dst: Reg, src: Reg, width: BitWidth, signed: bool) -> MachineFunction {
    let mut function = MachineFunction::new("extend");
    function.instructions = vec![MachineInstr::Extend {
        dst,
        src,
        width,
        signed,
    }];
    if dst != Reg::Rax {
        function.instructions.push(MachineInstr::Mov {
            dst: Reg::Rax,
            src: dst,
        });
    }
    function.instructions.push(MachineInstr::Ret);
    function
}

fn code(dst: Reg, src: Reg, width: BitWidth, signed: bool) -> Vec<u8> {
    let mut function = extend(dst, src, width, signed);
    function.instructions.truncate(1);
    function.instructions.push(MachineInstr::Ret);
    let bytes = function.emit_machine_code().unwrap();
    bytes[..bytes.len() - 1].to_vec()
}

#[test]
fn known_encodings() {
    use BitWidth::*;
    assert_eq!(code(Reg::Rax, Reg::Rcx, W8, false), [0x0f, 0xb6, 0xc1]); // movzx eax, cl
    assert_eq!(code(Reg::Rax, Reg::Rsi, W8, false), [0x40, 0x0f, 0xb6, 0xc6]); // movzx eax, sil
    assert_eq!(code(Reg::R9, Reg::Rdi, W16, false), [0x44, 0x0f, 0xb7, 0xcf]); // movzx r9d, di
    assert_eq!(code(Reg::Rax, Reg::R15, W32, false), [0x44, 0x89, 0xf8]); // mov eax, r15d
    assert_eq!(code(Reg::Rax, Reg::Rdi, W8, true), [0x48, 0x0f, 0xbe, 0xc7]); // movsx rax, dil
    assert_eq!(code(Reg::R8, Reg::Rax, W16, true), [0x4c, 0x0f, 0xbf, 0xc0]); // movsx r8, ax
    assert_eq!(code(Reg::Rcx, Reg::R10, W32, true), [0x49, 0x63, 0xca]); // movsxd rcx, r10d
}

#[test]
fn evaluation_matches_rust_casts() {
    use BitWidth::*;
    for value in [0i64, 0x7f, 0x80, 0xff, 0x1_8000, -1, i64::MIN, 0x1234_5678_9abc_def0] {
        for (width, signed, expected) in [
            (W8, false, value as u8 as i64),
            (W16, false, value as u16 as i64),
            (W32, false, value as u32 as i64),
            (W8, true, value as i8 as i64),
            (W16, true, value as i16 as i64),
            (W32, true, value as i32 as i64),
        ] {
            let function = extend(Reg::R9, Reg::Rdi, width, signed);
            assert_eq!(
                eval::evaluate(&function, &[value]).unwrap(),
                expected,
                "{width:?} signed={signed} {value:#x}"
            );
        }
    }
}
