#![cfg(all(target_os = "linux", target_arch = "x86_64"))]

use std::process::Command;

use kfrc_x86::machine::x86::{Condition, MachineFunction, MachineInstr, Reg};

#[test]
fn combined_assembly_keeps_each_functions_branch_targets_local() {
    let functions: Vec<MachineFunction> = ["branch", "branch_1", "branch.1", "branch$1"]
        .into_iter()
        .enumerate()
        .map(|(index, name)| {
            let mut function = MachineFunction::new(name);
            function.instructions = vec![
                MachineInstr::LoadArg {
                    dst: Reg::Rax,
                    index: 0,
                },
                MachineInstr::Test { reg: Reg::Rax },
                MachineInstr::Jcc {
                    condition: Condition::Eq,
                    target: 0,
                },
                MachineInstr::MovImm {
                    dst: Reg::Rax,
                    value: index as i64 * 10 + 1,
                },
                MachineInstr::Jmp(1),
                MachineInstr::Label(0),
                MachineInstr::MovImm {
                    dst: Reg::Rax,
                    value: index as i64 * 10,
                },
                MachineInstr::Label(1),
                MachineInstr::Ret,
            ];
            function
        })
        .collect();
    let assembly = functions
        .iter()
        .map(|function| function.emit_gnu_assembly().unwrap())
        .collect::<Vec<_>>()
        .join("\n");
    let dir = std::env::temp_dir().join(format!(
        "kfrc-assembly-labels-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir(&dir).unwrap();
    std::fs::write(dir.join("branches.s"), assembly).unwrap();
    std::fs::write(
        dir.join("driver.c"),
        r#"
#include <stdint.h>
extern uint64_t f0(uint64_t) __asm__("branch");
extern uint64_t f1(uint64_t) __asm__("branch_1");
extern uint64_t f2(uint64_t) __asm__("branch.1");
extern uint64_t f3(uint64_t) __asm__("branch$1");
int main(void) {
    uint64_t (*functions[])(uint64_t) = {f0,f1,f2,f3};
    for (unsigned i = 0; i < 4; i++) {
        if (functions[i](0) != i*10 || functions[i](1) != i*10+1 ||
            functions[i](UINT64_MAX) != i*10+1) return 1;
    }
    return 0;
}
"#,
    )
    .unwrap();
    let link = Command::new("cc")
        .current_dir(&dir)
        .args(["driver.c", "branches.s", "-o", "branches"])
        .output()
        .unwrap();
    assert!(
        link.status.success(),
        "{}",
        String::from_utf8_lossy(&link.stderr)
    );
    let run = Command::new(dir.join("branches")).output().unwrap();
    assert!(
        run.status.success(),
        "{:?}: {}",
        run.status,
        String::from_utf8_lossy(&run.stderr)
    );
    std::fs::remove_dir_all(dir).unwrap();
}
