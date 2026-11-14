use super::*;
use crate::{FunctionCode, SymbolBinding, write_object};
use kfrc_body::{FunctionAbi, RecordedTypes};
use std::process::Command;

fn native_recipes(name: &str) -> MachineFunction {
    use MachineInstr::*;
    let mut function = MachineFunction::new(name);
    function.stack_slots = 10;
    function.symbols = vec![
        "rematerialize_object".into(),
        "rematerialize_clobber".into(),
    ];
    function.instructions = vec![
        StackAddress {
            dst: Reg::Rax,
            offset: -64,
        },
        StoreIndirect {
            address: Reg::Rax,
            offset: 0,
            src: Reg::Rdi,
            bytes: 8,
        },
        Store {
            slot: StackSlot(0),
            src: Reg::Rax,
        },
        MovImm {
            dst: Reg::Rax,
            value: 17,
        },
        Store {
            slot: StackSlot(1),
            src: Reg::Rax,
        },
        SymbolAddress {
            dst: Reg::Rax,
            symbol: 0,
            addend: 8,
        },
        Store {
            slot: StackSlot(2),
            src: Reg::Rax,
        },
        Call { target: 1 },
        Load {
            dst: Reg::Rcx,
            slot: StackSlot(0),
        },
        LoadIndirect {
            dst: Reg::Rax,
            address: Reg::Rcx,
            offset: 0,
            bytes: 8,
        },
        Load {
            dst: Reg::Rdx,
            slot: StackSlot(1),
        },
        Add {
            dst: Reg::Rax,
            src: MachineOperand::Reg(Reg::Rdx),
        },
        Load {
            dst: Reg::Rcx,
            slot: StackSlot(2),
        },
        LoadIndirect {
            dst: Reg::Rdx,
            address: Reg::Rcx,
            offset: 0,
            bytes: 8,
        },
        Xor {
            dst: Reg::Rax,
            src: MachineOperand::Reg(Reg::Rdx),
        },
        Cmp {
            lhs: Reg::Rax,
            rhs: MachineOperand::Imm(7),
        },
        Load {
            dst: Reg::Rdx,
            slot: StackSlot(1),
        },
        Jcc {
            condition: Condition::Eq,
            target: 1,
        },
        Add {
            dst: Reg::Rax,
            src: MachineOperand::Reg(Reg::Rdx),
        },
        Ret,
        Label(1),
        Sub {
            dst: Reg::Rax,
            src: MachineOperand::Reg(Reg::Rdx),
        },
        Ret,
    ];
    function
}

fn pressure() -> MachineFunction {
    let scalar = LowerInstr::Scalar;
    let v = |index| VirtualValue::Reg(VirtualReg(index));
    let mut code = vec![scalar(VirtualInstr::LoadArg {
        dst: VirtualReg(0),
        index: 0,
    })];
    for slot in 0..24 {
        code.push(LowerInstr::StackAddress {
            dst: VirtualReg(slot + 1),
            slot: slot as usize,
        });
        code.push(LowerInstr::Store {
            address: v(slot + 1),
            offset: 0,
            src: v(0),
            bytes: 8,
        });
    }
    code.push(scalar(VirtualInstr::Call {
        dst: VirtualReg(25),
        target: "rematerialize_clobber".into(),
        args: vec![v(0)],
    }));
    let mut result = v(25);
    for slot in 0..24 {
        code.push(LowerInstr::Load {
            dst: VirtualReg(26 + slot),
            address: v(slot + 1),
            offset: 0,
            bytes: 8,
        });
        code.push(scalar(VirtualInstr::Add {
            dst: VirtualReg(50 + slot),
            lhs: result,
            rhs: v(26 + slot),
        }));
        result = v(50 + slot);
    }
    code.push(scalar(VirtualInstr::Return(result)));
    let graph = FunctionGraph::new(
        "rematerialize_pressure",
        FunctionAbi {
            args: vec![],
            ret: PassMode::Direct,
            c_variadic: false,
        },
        RecordedTypes::new(),
    );
    let function =
        crate::lower::allocated::compile(&graph, code, 74, &[SizeAlign { size: 8, align: 8 }; 24])
            .unwrap();
    assert!(
        function.stack_slots > 24,
        "fixture must force actual spilling"
    );
    assert!(
        function
            .instructions
            .iter()
            .filter(|i| matches!(i, MachineInstr::StackAddress { .. }))
            .count()
            > 24,
        "production allocation must recreate at least one spilled address"
    );
    function
}

#[test]
fn encoded_recipes_and_production_spills_survive_calls_and_memory_changes() {
    let original = native_recipes("rematerialize_original");
    let mut candidate = original.clone();
    candidate.name = "rematerialize_candidate".into();
    let stats = run(&mut candidate, 3);
    assert_eq!(
        (stats.slots, stats.loads_recreated, stats.stores_removed),
        (3, 4, 3)
    );
    assert_eq!(candidate.stack_slots, original.stack_slots);
    let object = write_object(
        &[original, candidate, pressure()]
            .into_iter()
            .map(|function| FunctionCode {
                function,
                binding: SymbolBinding::Global { hidden: false },
            })
            .collect::<Vec<_>>(),
    )
    .unwrap();
    let directory = std::env::temp_dir().join(format!(
        "kfrc-rematerialize-native-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir(&directory).unwrap();
    std::fs::write(directory.join("recipes.o"), object).unwrap();
    std::fs::write(directory.join("driver.c"), r#"
#include <stdint.h>
uint64_t rematerialize_object[2];
extern uint64_t rematerialize_original(uint64_t);
extern uint64_t rematerialize_candidate(uint64_t);
extern uint64_t rematerialize_pressure(uint64_t);
__attribute__((noinline)) uint64_t rematerialize_clobber(uint64_t x) {
    rematerialize_object[1] = x ^ UINT64_C(0xfedcba9876543210);
    uint64_t result = x ^ UINT64_C(0x123456789abcdef0);
    __asm__ volatile("xor %%rcx,%%rcx; xor %%rdx,%%rdx; xor %%rsi,%%rsi; xor %%rdi,%%rdi; xor %%r8,%%r8; xor %%r9,%%r9; xor %%r10,%%r10; xor %%r11,%%r11"
        ::: "rcx","rdx","rsi","rdi","r8","r9","r10","r11","cc");
    return result;
}
int main(void) {
    uint64_t state = UINT64_C(0x76543210fedcba98);
    for (unsigned j = 0; j < 10006; ++j) {
        uint64_t x = j < 6 ? (uint64_t[]){0,1,UINT64_MAX,UINT64_C(0x8000000000000000),UINT64_C(0x7fffffffffffffff),255}[j] : state;
        state ^= state << 13; state ^= state >> 7; state ^= state << 17;
        uint64_t mutated = x ^ UINT64_C(0xfedcba9876543210);
        uint64_t expected = (x + 17) ^ mutated;
        expected = expected == 7 ? expected - 17 : expected + 17;
        rematerialize_object[0] = UINT64_C(0x1111222233334444);
        rematerialize_object[1] = ~mutated;
        if (rematerialize_original(x) != expected) return 1;
        if (rematerialize_object[0] != UINT64_C(0x1111222233334444) || rematerialize_object[1] != mutated) return 2;
        rematerialize_object[1] = ~mutated;
        if (rematerialize_candidate(x) != expected) return 3;
        if (rematerialize_object[0] != UINT64_C(0x1111222233334444) || rematerialize_object[1] != mutated) return 4;
        if (rematerialize_pressure(x) != (x ^ UINT64_C(0x123456789abcdef0)) + 24 * x) return 5;
        if (rematerialize_object[0] != UINT64_C(0x1111222233334444) || rematerialize_object[1] != mutated) return 6;
    }
    return 0;
}
"#).unwrap();
    let linked = Command::new("cc")
        .current_dir(&directory)
        .args(["-O2", "driver.c", "recipes.o", "-o", "driver"])
        .output()
        .unwrap();
    assert!(
        linked.status.success(),
        "{}",
        String::from_utf8_lossy(&linked.stderr)
    );
    let result = Command::new(directory.join("driver")).output().unwrap();
    assert!(result.status.success(), "{result:?}");
    std::fs::remove_dir_all(directory).unwrap();
}
