use super::*;
use crate::machine::eval;
use kfrc_body::{FunctionAbi, RecordedTypes};

fn v(index: u32) -> VirtualValue {
    VirtualValue::Reg(VirtualReg(index))
}

fn scalar(instruction: VirtualInstr) -> LowerInstr {
    LowerInstr::Scalar(instruction)
}

fn compile_stream(name: &str, code: Vec<LowerInstr>, count: u32) -> MachineFunction {
    let graph = FunctionGraph::new(
        name,
        FunctionAbi {
            args: vec![],
            ret: PassMode::Direct,
            c_variadic: false,
        },
        RecordedTypes::new(),
    );
    compile(&graph, code, count, &[]).unwrap()
}

fn product(factor: i64, constant_left: bool, width: Option<(BitWidth, bool)>) -> MachineFunction {
    let mut code = vec![scalar(VirtualInstr::LoadArg {
        dst: VirtualReg(0),
        index: 0,
    })];
    if let Some((width, signed)) = width {
        code.push(LowerInstr::Extend {
            dst: VirtualReg(1),
            value: VirtualReg(0),
            width,
            signed,
        });
    } else {
        code.push(scalar(VirtualInstr::Mov {
            dst: VirtualReg(1),
            src: v(0),
        }));
    }
    let (lhs, rhs) = if constant_left {
        (VirtualValue::Imm(factor), v(1))
    } else {
        (v(1), VirtualValue::Imm(factor))
    };
    code.push(scalar(VirtualInstr::Mul {
        dst: VirtualReg(2),
        lhs,
        rhs,
    }));
    if let Some((width, signed)) = width {
        code.push(LowerInstr::Extend {
            dst: VirtualReg(3),
            value: VirtualReg(2),
            width,
            signed,
        });
    } else {
        code.push(scalar(VirtualInstr::Mov {
            dst: VirtualReg(3),
            src: v(2),
        }));
    }
    // The original argument stays live: a tied multiply must not overwrite it.
    code.push(scalar(VirtualInstr::Xor {
        dst: VirtualReg(4),
        lhs: v(3),
        rhs: v(0),
    }));
    code.push(scalar(VirtualInstr::Return(v(4))));
    compile_stream("small_product", code, 5)
}

fn inputs() -> Vec<i64> {
    let mut values = vec![
        i64::MIN,
        i64::MAX,
        -1,
        0,
        1,
        127,
        128,
        255,
        256,
        65535,
        65536,
    ];
    let mut state = 0x47a9_391d_e5c0_168bu64;
    for _ in 0..256 {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        values.push(state as i64);
    }
    values
}

#[test]
fn small_products_preserve_live_inputs_and_narrow_wrapping() {
    for factor in [3, 5, 9] {
        for constant_left in [false, true] {
            for width in [
                None,
                Some((BitWidth::W8, false)),
                Some((BitWidth::W8, true)),
                Some((BitWidth::W16, false)),
                Some((BitWidth::W16, true)),
                Some((BitWidth::W32, false)),
                Some((BitWidth::W32, true)),
            ] {
                let function = product(factor, constant_left, width);
                assert_eq!(
                    function
                        .instructions
                        .iter()
                        .filter(|i| matches!(i, MachineInstr::EffectiveAddress { .. }))
                        .count(),
                    1
                );
                assert!(
                    !function
                        .instructions
                        .iter()
                        .any(|i| matches!(i, MachineInstr::IMul { .. }))
                );
                function.emit_machine_code().unwrap();
                for x in inputs() {
                    let result = x.wrapping_mul(factor);
                    let result = match width {
                        None => result,
                        Some((BitWidth::W8, false)) => i64::from(result as u8),
                        Some((BitWidth::W8, true)) => i64::from(result as i8),
                        Some((BitWidth::W16, false)) => i64::from(result as u16),
                        Some((BitWidth::W16, true)) => i64::from(result as i16),
                        Some((BitWidth::W32, false)) => i64::from(result as u32),
                        Some((BitWidth::W32, true)) => i64::from(result as i32),
                        _ => unreachable!(),
                    };
                    assert_eq!(
                        eval::evaluate(&function, &[x]).unwrap(),
                        result ^ x,
                        "factor={factor} left={constant_left} width={width:?} x={x}"
                    );
                }
            }
        }
    }
}

#[test]
fn unsupported_factors_retain_multiplication() {
    for factor in [-9, -5, -3, 0, 1, 2, 4, 7, 8, 10, i64::MAX] {
        let function = product(factor, false, None);
        assert!(
            function
                .instructions
                .iter()
                .any(|i| matches!(i, MachineInstr::IMul { .. }))
        );
        for x in inputs() {
            assert_eq!(
                eval::evaluate(&function, &[x]).unwrap(),
                x.wrapping_mul(factor) ^ x
            );
        }
    }
}

#[test]
fn effective_address_keeps_flags_and_handles_overflow_and_register_aliases() {
    for scale in [Scale::One, Scale::Two, Scale::Four, Scale::Eight] {
        for offset in [i32::MIN, -128, 0, 127, i32::MAX] {
            for dst in [Reg::Rax, Reg::Rdi, Reg::Rsi, Reg::R12] {
                let mut function = MachineFunction::new("lea_flags");
                function.instructions = vec![
                    MachineInstr::Cmp {
                        lhs: Reg::Rdi,
                        rhs: MachineOperand::Imm(7),
                    },
                    MachineInstr::EffectiveAddress {
                        dst,
                        address: MemoryAddress {
                            base: Reg::Rdi,
                            index: Some((Reg::Rsi, scale)),
                            offset,
                        },
                    },
                    MachineInstr::Jcc {
                        condition: Condition::Eq,
                        target: 1,
                    },
                    MachineInstr::MovImm {
                        dst: Reg::Rax,
                        value: -1,
                    },
                    MachineInstr::Ret,
                    MachineInstr::Label(1),
                    MachineInstr::Mov {
                        dst: Reg::Rax,
                        src: dst,
                    },
                    MachineInstr::Ret,
                ];
                for x in [6i64, 7] {
                    for y in [i64::MIN, i64::MAX, -1, 0, 1] {
                        let expected = if x == 7 {
                            x.wrapping_add(y.wrapping_mul(i64::from(scale.factor())))
                                .wrapping_add(i64::from(offset))
                        } else {
                            -1
                        };
                        assert_eq!(eval::evaluate(&function, &[x, y]).unwrap(), expected);
                    }
                }
            }
        }
    }
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[test]
fn encoded_products_survive_real_call_clobbers_and_spills() {
    use crate::{FunctionCode, SymbolBinding, write_object};
    use std::process::Command;
    let mut code = vec![scalar(VirtualInstr::LoadArg {
        dst: VirtualReg(0),
        index: 0,
    })];
    for i in 0..24 {
        code.push(scalar(VirtualInstr::Add {
            dst: VirtualReg(1 + i * 2),
            lhs: v(0),
            rhs: VirtualValue::Imm(i as i64),
        }));
        code.push(scalar(VirtualInstr::Mul {
            dst: VirtualReg(2 + i * 2),
            lhs: v(1 + i * 2),
            rhs: VirtualValue::Imm([3, 5, 9][i as usize % 3]),
        }));
    }
    code.push(scalar(VirtualInstr::Call {
        dst: VirtualReg(49),
        target: "product_clobber".into(),
        args: vec![v(0)],
    }));
    let mut sum = v(49);
    for i in 0..24 {
        code.push(scalar(VirtualInstr::Add {
            dst: VirtualReg(50 + i),
            lhs: sum,
            rhs: v(2 + i * 2),
        }));
        sum = v(50 + i);
    }
    code.push(scalar(VirtualInstr::Return(sum)));
    let function = compile_stream("product_pressure", code, 74);
    assert!(function.stack_slots > 0);
    assert_eq!(
        function
            .instructions
            .iter()
            .filter(|i| matches!(i, MachineInstr::EffectiveAddress { .. }))
            .count(),
        24
    );
    let object = write_object(&[FunctionCode {
        function,
        binding: SymbolBinding::Global { hidden: false },
    }])
    .unwrap();
    let dir = std::env::temp_dir().join(format!(
        "kfrc-product-native-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir(&dir).unwrap();
    std::fs::write(dir.join("product.o"), object).unwrap();
    std::fs::write(dir.join("driver.c"), r#"
#include <stdint.h>
extern uint64_t product_pressure(uint64_t);
__attribute__((noinline)) uint64_t product_clobber(uint64_t x) {
    uint64_t result = x ^ UINT64_C(0x123456789abcdef0);
    __asm__ volatile("xor %%rcx,%%rcx; xor %%rdx,%%rdx; xor %%rsi,%%rsi; xor %%rdi,%%rdi; xor %%r8,%%r8; xor %%r9,%%r9; xor %%r10,%%r10; xor %%r11,%%r11"
        ::: "rcx","rdx","rsi","rdi","r8","r9","r10","r11","cc");
    return result;
}
int main(void) {
    uint64_t state = UINT64_C(0x98765432fedcba10);
    const uint64_t factors[3] = {3, 5, 9};
    for (unsigned j = 0; j < 10006; ++j) {
        uint64_t x = j < 6 ? (uint64_t[]){0,1,UINT64_MAX,UINT64_C(0x8000000000000000),UINT64_C(0x7fffffffffffffff),255}[j] : state;
        state ^= state << 13; state ^= state >> 7; state ^= state << 17;
        uint64_t expected = x ^ UINT64_C(0x123456789abcdef0);
        for (unsigned i = 0; i < 24; ++i) expected += (x + i) * factors[i % 3];
        if (product_pressure(x) != expected) return 1;
    }
    return 0;
}
"#).unwrap();
    let linked = Command::new("cc")
        .current_dir(&dir)
        .args(["-O2", "driver.c", "product.o", "-o", "driver"])
        .output()
        .unwrap();
    assert!(
        linked.status.success(),
        "{}",
        String::from_utf8_lossy(&linked.stderr)
    );
    let result = Command::new(dir.join("driver")).output().unwrap();
    assert!(result.status.success(), "{result:?}");
    std::fs::remove_dir_all(dir).unwrap();
}
