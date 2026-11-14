//! Execute encoded masked TESTs across operand widths and byte-register forms.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
use kfrc_x86::machine::x86::{BitWidth, Condition, MachineFunction, MachineInstr as I, Reg};
use kfrc_x86::{FunctionCode, SymbolBinding, write_object};

#[test]
fn masked_test_encodings_execute_with_live_source_registers() {
    let dir = std::env::temp_dir().join(format!(
        "kfrc-masked-tests-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos(),
    ));
    std::fs::create_dir(&dir).unwrap();
    let masks = [
        (3u32, BitWidth::W8, 3u64),
        (0x80, BitWidth::W8, 0x80),
        (0x8100, BitWidth::W16, 0x8100),
        (0x8000_0000, BitWidth::W32, 0x8000_0000),
        (u32::MAX, BitWidth::W32, u32::MAX as u64),
        (0x8000_0000, BitWidth::W64, 0xffff_ffff_8000_0000),
        (u32::MAX, BitWidth::W64, u64::MAX),
    ];
    let mut functions = Vec::new();
    let mut declarations = String::new();
    let mut checks = String::new();
    for reg in [Reg::Rax, Reg::Rsi, Reg::Rdi, Reg::R8, Reg::R12, Reg::R15] {
        for (mask, width, expected) in masks {
            for condition in [Condition::Eq, Condition::NotEq] {
                let symbol = format!("masked_{}", functions.len());
                let mut function = MachineFunction::new(&symbol);
                if Reg::CALLEE_SAVED.contains(&reg) {
                    function.callee_saved.push(reg);
                }
                function.instructions = vec![
                    I::LoadArg { dst: reg, index: 0 },
                    I::TestMasked { reg, mask, width },
                    I::Jcc {
                        condition,
                        target: 1,
                    },
                    I::Mov {
                        dst: Reg::Rax,
                        src: reg,
                    },
                    I::Not { dst: Reg::Rax },
                    I::Ret,
                    I::Label(1),
                    I::Mov {
                        dst: Reg::Rax,
                        src: reg,
                    },
                    I::Ret,
                ];
                declarations.push_str(&format!("extern uint64_t {symbol}(uint64_t);\n"));
                checks.push_str(&format!(
                    "if ({symbol}(x) != (((x & UINT64_C({expected})) {} 0) ? x : ~x)) return {};\n",
                    if condition == Condition::Eq {
                        "=="
                    } else {
                        "!="
                    },
                    functions.len() + 1,
                ));
                functions.push(FunctionCode {
                    function,
                    binding: SymbolBinding::Global { hidden: false },
                });
            }
        }
    }
    std::fs::write(dir.join("masked.o"), write_object(&functions).unwrap()).unwrap();
    let driver = format!(
        "#include <stdint.h>\n{declarations}\nint main(void) {{\nuint64_t state=UINT64_C(0x197bcda0536802ff);\nfor(unsigned i=0;i<1006;i++) {{\nuint64_t x;\nif(i==0) x=0;\nelse if(i==1) x=UINT64_MAX;\nelse if(i<66) x=UINT64_C(1)<<(i-2);\nelse if(i<130) x=~(UINT64_C(1)<<(i-66));\nelse {{ state^=state<<13;state^=state>>7;state^=state<<17;x=state; }}\n{checks}\n}}\nreturn 0;\n}}\n"
    );
    std::fs::write(dir.join("driver.c"), driver).unwrap();
    let output = std::process::Command::new("cc")
        .current_dir(&dir)
        .args(["driver.c", "masked.o", "-o", "driver"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let status = std::process::Command::new(dir.join("driver"))
        .status()
        .unwrap();
    assert!(status.success(), "{status}");
}
