//! Linked ELF validation for aligned loops, including relocated calls in the body.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
use kfrc_x86::machine::x86::{Condition, MachineFunction, MachineInstr as I, Operand, Reg};
use kfrc_x86::{FunctionCode, SymbolBinding, write_object};
use object::{Object, ObjectSection};

#[test]
fn aligned_loops_and_calls_execute_after_linking() {
    let mut function = MachineFunction::new("loop_calls");
    function.callee_saved = vec![Reg::Rbx, Reg::R12];
    function.symbols = vec!["seven".into()];
    function.loop_headers = vec![1];
    function.instructions = vec![
        I::LoadArg {
            dst: Reg::Rbx,
            index: 0,
        },
        I::MovImm {
            dst: Reg::R12,
            value: 0,
        },
        I::Cmp {
            lhs: Reg::Rbx,
            rhs: Operand::Imm(0),
        },
        I::Jcc {
            condition: Condition::Eq,
            target: 2,
        },
        I::Label(1),
        I::Call { target: 0 },
        I::Add {
            dst: Reg::R12,
            src: Operand::Reg(Reg::Rax),
        },
        I::Sub {
            dst: Reg::Rbx,
            src: Operand::Imm(1),
        },
        I::Cmp {
            lhs: Reg::Rbx,
            rhs: Operand::Imm(0),
        },
        I::Jcc {
            condition: Condition::NotEq,
            target: 1,
        },
        I::Label(2),
        I::Mov {
            dst: Reg::Rax,
            src: Reg::R12,
        },
        I::Ret,
    ];
    let bytes = write_object(&[FunctionCode {
        function,
        binding: SymbolBinding::Global { hidden: false },
    }])
    .unwrap();
    let file = object::File::parse(bytes.as_slice()).unwrap();
    let section = file.section_by_name(".text.loop_calls").unwrap();
    let alignment = section.align();
    assert!(matches!(alignment, 16 | 32 | 64));
    let code = section.data().unwrap();
    let calls: Vec<_> = section.relocations().collect();
    assert_eq!(calls.len(), 1);
    // This loop begins with its call, and every backedge skips the preceding NOPs.
    let header = calls[0].0 - 1;
    assert_eq!(header % alignment, 0);
    assert_eq!(code[header as usize], 0xe8);
    let dir = std::env::temp_dir().join(format!(
        "kfrc-loop-alignment-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir(&dir).unwrap();
    std::fs::write(dir.join("loop.o"), bytes).unwrap();
    std::fs::write(
        dir.join("driver.c"),
        r#"
#include <stdint.h>
uint64_t seven(void) { return 7; }
extern uint64_t loop_calls(uint64_t);
int main(void) {
  for(uint64_t n=0;n<1000;n++) if(loop_calls(n)!=7*n) return 1;
  return 0;
}
"#,
    )
    .unwrap();
    let output = std::process::Command::new("cc")
        .current_dir(&dir)
        .args(["driver.c", "loop.o", "-o", "driver"])
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
