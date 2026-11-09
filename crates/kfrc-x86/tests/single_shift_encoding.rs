use kfrc_x86::machine::x86::{MachineFunction, MachineInstr, Reg};

const REGISTERS: [Reg; 3] = [Reg::Rax, Reg::R8, Reg::R15];
const DIRECTIONS: [Direction; 3] = [
    Direction::Left,
    Direction::ArithmeticRight,
    Direction::LogicalRight,
];

#[derive(Clone, Copy, Debug)]
enum Direction {
    Left,
    ArithmeticRight,
    LogicalRight,
}

fn shift(direction: Direction, dst: Reg, count: u8) -> MachineInstr {
    match direction {
        Direction::Left => MachineInstr::ShlImm { dst, count },
        Direction::ArithmeticRight => MachineInstr::ShrImm { dst, count },
        Direction::LogicalRight => MachineInstr::LShrImm { dst, count },
    }
}

fn encode_shift(reg: Reg, direction: Direction, count: u8) -> Vec<u8> {
    let mut function = MachineFunction::new("single_shift");
    function.instructions = vec![shift(direction, reg, count), MachineInstr::Ret];
    let code = function.emit_machine_code().unwrap();
    code[4..code.len() - 2].to_vec()
}

fn expected_encoding(reg: Reg, direction: Direction, count: u8) -> Vec<u8> {
    let (rex, low_code) = match reg {
        Reg::Rax => (0x48, 0),
        Reg::R8 => (0x49, 0),
        Reg::R15 => (0x49, 7),
        _ => unreachable!(),
    };
    let extension = match direction {
        Direction::Left => 4,
        Direction::ArithmeticRight => 7,
        Direction::LogicalRight => 5,
    };
    let mut expected = vec![
        rex,
        if count == 1 { 0xd1 } else { 0xc1 },
        0xc0 | (extension << 3) | low_code,
    ];
    if count != 1 {
        expected.push(count);
    }
    expected
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn expected_result(direction: Direction, value: i64, count: u8) -> i64 {
    let count = u32::from(count) & 63;
    match direction {
        Direction::Left => value.wrapping_shl(count),
        Direction::ArithmeticRight => value.wrapping_shr(count),
        Direction::LogicalRight => (value as u64).wrapping_shr(count) as i64,
    }
}

#[test]
fn count_one_uses_d1_for_every_register_and_direction() {
    for reg in REGISTERS {
        for direction in DIRECTIONS {
            let expected = expected_encoding(reg, direction, 1);
            assert_eq!(expected.len(), 3);
            assert_eq!(
                encode_shift(reg, direction, 1),
                expected,
                "{reg:?} {direction:?}"
            );
        }
    }
}

#[test]
fn zero_sixty_three_and_sixty_four_keep_c1_imm8() {
    for reg in REGISTERS {
        for direction in DIRECTIONS {
            for count in [0, 63, 64] {
                let expected = expected_encoding(reg, direction, count);
                assert_eq!(expected.len(), 4);
                assert_eq!(
                    encode_shift(reg, direction, count),
                    expected,
                    "{reg:?} {direction:?} count {count}"
                );
            }
        }
    }
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
mod direct_execution {
    use super::*;
    use kfrc_x86::{FunctionCode, SymbolBinding, write_object};
    use std::env;
    use std::fs;
    use std::path::PathBuf;
    use std::process::{self, Command};
    use std::sync::atomic::{AtomicU64, Ordering};

    const VALUES: [i64; 5] = [0, 1, -2, i64::MIN, 0x0123_4567_89ab_cdef];

    #[test]
    fn linked_object_executes_every_encoding_with_hardware_count_masking() {
        let Some(linker) = ["cc", "gcc", "clang"].into_iter().find(|command| {
            Command::new(command)
                .arg("--version")
                .output()
                .is_ok_and(|output| output.status.success())
        }) else {
            eprintln!("C linker not found; skipping direct object execution");
            return;
        };

        let mut functions = Vec::new();
        let mut declarations = String::from("#include <stdint.h>\n");
        let mut checks = String::from("int main(void) {\n");
        let mut function_number = 0;
        let mut check_number = 1;
        for reg in REGISTERS {
            for direction in DIRECTIONS {
                for count in [0, 1, 63, 64] {
                    let name = format!("shift_{function_number}");
                    function_number += 1;
                    let mut function = MachineFunction::new(&name);
                    function.instructions = vec![
                        MachineInstr::LoadArg { dst: reg, index: 0 },
                        shift(direction, reg, count),
                    ];
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

                    functions.push(FunctionCode {
                        function,
                        binding: SymbolBinding::Global { hidden: false },
                    });
                    declarations.push_str(&format!("extern int64_t {name}(int64_t value);\n"));
                    for value in VALUES {
                        let expected = expected_result(direction, value, count);
                        checks.push_str(&format!(
                            "    if ({name}((int64_t)UINT64_C(0x{:016x})) != (int64_t)UINT64_C(0x{:016x})) return {check_number};\n",
                            value as u64, expected as u64
                        ));
                        check_number += 1;
                    }
                }
            }
        }
        checks.push_str("    return 0;\n}\n");

        let directory = TempDirectory::new();
        let object = directory.0.join("single_shifts.o");
        let harness = directory.0.join("single_shifts.c");
        let executable = directory.0.join("single_shifts");
        fs::write(&object, write_object(&functions).unwrap()).unwrap();
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
            let path = env::temp_dir().join(format!("kfrc-single-shift-{}-{id}", process::id()));
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
