//! Labels consume no instructions, so jumps to the next label are redundant.
use super::MachineInstr;
use kfrc_body::fx_hash::HashSet;

pub(super) fn remove_fallthrough_jumps(instructions: &mut Vec<MachineInstr>) {
    let mut next_labels = HashSet::default();
    let mut retained = Vec::with_capacity(instructions.len());
    for instruction in instructions.drain(..).rev() {
        match &instruction {
            MachineInstr::Label(label) => {
                next_labels.insert(*label);
            }
            MachineInstr::Jmp(target) if next_labels.contains(target) => continue,
            _ => next_labels.clear(),
        }
        retained.push(instruction);
    }
    retained.reverse();
    *instructions = retained;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::machine::x86::{Condition, MachineFunction, Operand, Reg};

    #[test]
    fn consecutive_labels_and_redundant_chains_fall_through() {
        let mut instructions = vec![
            MachineInstr::Jmp(2),
            MachineInstr::Jmp(1),
            MachineInstr::Label(1),
            MachineInstr::Label(2),
            MachineInstr::Ret,
        ];
        remove_fallthrough_jumps(&mut instructions);
        assert_eq!(
            instructions,
            vec![
                MachineInstr::Label(1),
                MachineInstr::Label(2),
                MachineInstr::Ret
            ]
        );
    }

    #[test]
    fn real_edges_loops_and_flags_consumers_remain() {
        let mut instructions = vec![
            MachineInstr::Label(1),
            MachineInstr::Jmp(1),
            MachineInstr::Jmp(2),
            MachineInstr::Mov {
                dst: Reg::Rax,
                src: Reg::Rbx,
            },
            MachineInstr::Jcc {
                condition: Condition::Eq,
                target: 2,
            },
            MachineInstr::Label(2),
            MachineInstr::Ret,
        ];
        let expected = instructions.clone();
        remove_fallthrough_jumps(&mut instructions);
        assert_eq!(instructions, expected);
    }

    fn counted_sum(name: &str, external_call: bool) -> MachineFunction {
        let mut function = MachineFunction::new(name);
        if external_call {
            function.symbols.push("fallthrough_identity".into());
            function.instructions.extend([
                MachineInstr::LoadArg {
                    dst: Reg::Rdi,
                    index: 0,
                },
                MachineInstr::Call { target: 0 },
                MachineInstr::Mov {
                    dst: Reg::Rcx,
                    src: Reg::Rax,
                },
            ]);
        } else {
            function.instructions.push(MachineInstr::LoadArg {
                dst: Reg::Rcx,
                index: 0,
            });
        }
        function.instructions.extend([
            MachineInstr::MovImm {
                dst: Reg::Rax,
                value: 0,
            },
            MachineInstr::Label(10),
            MachineInstr::Cmp {
                lhs: Reg::Rcx,
                rhs: Operand::Imm(0),
            },
            MachineInstr::Jmp(12),
            MachineInstr::Label(11),
            MachineInstr::Label(12),
            MachineInstr::Jcc {
                condition: Condition::Eq,
                target: 20,
            },
            MachineInstr::Add {
                dst: Reg::Rax,
                src: Operand::Reg(Reg::Rcx),
            },
            MachineInstr::Sub {
                dst: Reg::Rcx,
                src: Operand::Imm(1),
            },
            MachineInstr::Jmp(10),
            MachineInstr::Label(20),
            MachineInstr::Ret,
        ]);
        function
    }

    #[test]
    fn flags_and_loop_backedges_have_identical_machine_results() {
        let before = counted_sum("sum", false);
        let mut after = before.clone();
        remove_fallthrough_jumps(&mut after.instructions);
        assert_eq!(before.instructions.len(), after.instructions.len() + 1);
        for count in 0..64 {
            let expected = count * (count + 1) / 2;
            assert_eq!(
                crate::machine::eval::evaluate(&before, &[count]).unwrap(),
                expected
            );
            assert_eq!(
                crate::machine::eval::evaluate(&after, &[count]).unwrap(),
                expected
            );
        }
    }

    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    #[test]
    fn encoded_fallthrough_preserves_call_return_flags_and_loop_edges() {
        use crate::{FunctionCode, SymbolBinding, write_object};
        use std::process::Command;
        let before = counted_sum("sum_before", true);
        let mut after = counted_sum("sum_after", true);
        remove_fallthrough_jumps(&mut after.instructions);
        let functions = [before, after].map(|function| FunctionCode {
            function,
            binding: SymbolBinding::Global { hidden: false },
        });
        let directory =
            std::env::temp_dir().join(format!("kfrc-fallthrough-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let source = directory.join("main.c");
        let object = directory.join("sum.o");
        let executable = directory.join("check");
        std::fs::write(&object, write_object(&functions).unwrap()).unwrap();
        std::fs::write(
            &source,
            r#"
#include <stdint.h>
extern uint64_t sum_before(uint64_t), sum_after(uint64_t);
uint64_t fallthrough_identity(uint64_t value) {
    __asm__ volatile("cmp $1, %%rdi" ::: "cc");
    return value;
}
int main(void) {
    for (uint64_t n = 0; n < 256; ++n) {
        uint64_t expected = n * (n + 1) / 2;
        if (sum_before(n) != expected || sum_after(n) != expected) return 1;
    }
    return 0;
}
"#,
        )
        .unwrap();
        let linked = Command::new("cc")
            .arg("-O2")
            .arg(&source)
            .arg(&object)
            .arg("-o")
            .arg(&executable)
            .output()
            .unwrap();
        assert!(
            linked.status.success(),
            "{}",
            String::from_utf8_lossy(&linked.stderr)
        );
        let ran = Command::new(&executable).output().unwrap();
        assert!(
            ran.status.success(),
            "fallthrough machine mismatch {:?}",
            ran.status
        );
        std::fs::remove_dir_all(directory).unwrap();
    }
}
