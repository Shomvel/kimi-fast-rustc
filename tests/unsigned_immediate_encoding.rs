use kimi_fast_rustc::x86::{MachineFunction, MachineInstr, Reg};

const REGISTERS: [Reg; 3] = [Reg::Rax, Reg::R8, Reg::R15];

fn encode_immediate(reg: Reg, value: i64) -> Vec<u8> {
    let mut function = MachineFunction::new("immediate");
    function.instructions = vec![MachineInstr::MovImm { dst: reg, value }, MachineInstr::Ret];
    let code = function.emit_machine_code().unwrap();
    code[4..code.len() - 2].to_vec()
}

fn register_encoding(reg: Reg) -> (Option<u8>, u8, u8, u8) {
    match reg {
        Reg::Rax => (None, 0x48, 0xb8, 0xc0),
        Reg::R8 => (Some(0x41), 0x49, 0xb8, 0xc0),
        Reg::R15 => (Some(0x41), 0x49, 0xbf, 0xc7),
        _ => unreachable!(),
    }
}

#[test]
fn unsigned_imm32_uses_b8_with_zero_extension_and_rex_b() {
    for reg in REGISTERS {
        let (rex32, _, opcode, _) = register_encoding(reg);
        for (value, immediate) in [
            (0x8000_0000, [0x00, 0x00, 0x00, 0x80]),
            (0xffff_ffff, [0xff, 0xff, 0xff, 0xff]),
        ] {
            let mut expected = Vec::new();
            expected.extend(rex32);
            expected.push(opcode);
            expected.extend_from_slice(&immediate);
            assert_eq!(expected.len(), if reg == Reg::Rax { 5 } else { 6 });
            assert_eq!(
                encode_immediate(reg, value),
                expected,
                "{reg:?}, {value:#x}"
            );
        }
    }
}

#[test]
fn signed_imm32_and_imm64_boundaries_keep_their_encodings() {
    for reg in REGISTERS {
        let (_, rex64, opcode, modrm) = register_encoding(reg);
        for (value, expected_immediate) in [
            (0x7fff_ffff, &[0xff, 0xff, 0xff, 0x7f][..]),
            (-1, &[0xff, 0xff, 0xff, 0xff][..]),
        ] {
            let mut expected = vec![rex64, 0xc7, modrm];
            expected.extend_from_slice(expected_immediate);
            assert_eq!(encode_immediate(reg, value), expected, "{reg:?}, {value}");
        }

        let mut expected = vec![rex64, opcode];
        expected.extend_from_slice(&0x1_0000_0000_i64.to_le_bytes());
        assert_eq!(
            encode_immediate(reg, 0x1_0000_0000),
            expected,
            "{reg:?}, 0x100000000"
        );
    }
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
mod direct_execution {
    use super::*;
    use kimi_fast_rustc::elf::{FunctionSymbol, write_object};

    const VALUES: [i64; 5] = [0x7fff_ffff, 0x8000_0000, 0xffff_ffff, 0x1_0000_0000, -1];
    use std::env;
    use std::fs;
    use std::path::PathBuf;
    use std::process::{self, Command};
    use std::sync::atomic::{AtomicU64, Ordering};

    #[test]
    fn linked_object_returns_every_boundary_value_from_each_register() {
        let Some(linker) = ["cc", "gcc", "clang"].into_iter().find(|command| {
            Command::new(command)
                .arg("--version")
                .output()
                .is_ok_and(|output| output.status.success())
        }) else {
            eprintln!("C linker not found; skipping direct object execution");
            return;
        };

        let mut text = Vec::new();
        let mut symbols = Vec::new();
        let mut declarations = String::from("#include <stdint.h>\n");
        let mut checks = String::from("int main(void) {\n");
        let mut case_number = 1;
        for reg in REGISTERS {
            for value in VALUES {
                let name = format!("immediate_{case_number}");
                let mut function = MachineFunction::new(&name);
                function.instructions = vec![MachineInstr::MovImm { dst: reg, value }];
                if reg != Reg::Rax {
                    function.instructions.push(MachineInstr::Mov {
                        dst: Reg::Rax,
                        src: reg,
                    });
                }
                if reg == Reg::R15 {
                    function.callee_saved.push(Reg::R15);
                }
                function.instructions.push(MachineInstr::Ret);

                let code = function.emit_machine_code().unwrap();
                symbols.push(FunctionSymbol::new(
                    &name,
                    text.len() as u64,
                    code.len() as u64,
                ));
                text.extend_from_slice(&code);
                declarations.push_str(&format!("extern int64_t {name}(void);\n"));
                checks.push_str(&format!(
                    "    if ({name}() != {value}LL) return {case_number};\n"
                ));
                case_number += 1;
            }
        }
        checks.push_str("    return 0;\n}\n");

        let directory = TempDirectory::new();
        let object = directory.0.join("immediates.o");
        let harness = directory.0.join("immediates.c");
        let executable = directory.0.join("immediates");
        fs::write(&object, write_object(&text, &symbols).unwrap()).unwrap();
        fs::write(&harness, declarations + &checks).unwrap();

        let output = Command::new(linker)
            .arg(&object)
            .arg(&harness)
            .arg("-o")
            .arg(&executable)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{linker} failed:\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let output = Command::new(&executable).output().unwrap();
        assert!(
            output.status.success(),
            "direct object execution failed with status {}",
            output.status
        );
    }

    struct TempDirectory(PathBuf);

    impl TempDirectory {
        fn new() -> Self {
            static NEXT_ID: AtomicU64 = AtomicU64::new(0);
            let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
            let path =
                env::temp_dir().join(format!("kfrc-unsigned-immediate-{}-{id}", process::id()));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TempDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
}
