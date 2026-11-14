use super::*;
use crate::machine::eval;
use kfrc_body::{FunctionAbi, RecordedTypes};

fn v(index: u32) -> VirtualValue {
    VirtualValue::Reg(VirtualReg(index))
}
fn scalar(instruction: VirtualInstr) -> LowerInstr {
    LowerInstr::Scalar(instruction)
}
fn compile_stream(stream: Vec<LowerInstr>, count: u32) -> MachineFunction {
    let graph = FunctionGraph::new(
        "intrinsic_constraints",
        FunctionAbi {
            args: vec![],
            ret: PassMode::Direct,
            c_variadic: false,
        },
        RecordedTypes::new(),
    );
    compile(&graph, stream, count, &[]).unwrap()
}

#[test]
fn scan_fallback_survives_identical_source_and_live_through_uses() {
    for reverse in [false, true] {
        for shared in [false, true] {
            let function = compile_stream(
                vec![
                    scalar(VirtualInstr::LoadArg {
                        dst: VirtualReg(0),
                        index: 0,
                    }),
                    scalar(VirtualInstr::LoadArg {
                        dst: VirtualReg(1),
                        index: 1,
                    }),
                    LowerInstr::BitScanZero {
                        dst: VirtualReg(2),
                        value: v(0),
                        zero: if shared { v(0) } else { v(1) },
                        reverse,
                    },
                    scalar(VirtualInstr::Add {
                        dst: VirtualReg(3),
                        lhs: v(2),
                        rhs: v(0),
                    }),
                    scalar(VirtualInstr::Add {
                        dst: VirtualReg(4),
                        lhs: v(3),
                        rhs: v(1),
                    }),
                    scalar(VirtualInstr::Return(v(4))),
                ],
                5,
            );
            function.emit_machine_code().unwrap();
            for input in [0i64, 1, 16, i64::MIN, -1] {
                let fallback = if shared { input } else { 79 };
                let scan = if input == 0 {
                    fallback
                } else if reverse {
                    i64::from(63 - (input as u64).leading_zeros())
                } else {
                    i64::from((input as u64).trailing_zeros())
                };
                assert_eq!(
                    eval::evaluate(&function, &[input, 79]).unwrap(),
                    scan.wrapping_add(input).wrapping_add(79)
                );
            }
        }
    }
}

#[test]
fn repeated_cl_rotates_preserve_live_source_and_count() {
    for shared in [false, true] {
        let mut code = vec![
            scalar(VirtualInstr::LoadArg {
                dst: VirtualReg(0),
                index: 0,
            }),
            scalar(VirtualInstr::LoadArg {
                dst: VirtualReg(1),
                index: 1,
            }),
        ];
        let amount = if shared { v(0) } else { v(1) };
        let mut value = v(0);
        for index in 2..10 {
            code.push(LowerInstr::Rotate {
                dst: VirtualReg(index),
                value,
                amount,
                width: BitWidth::W64,
                right: false,
            });
            value = v(index);
        }
        code.extend([
            scalar(VirtualInstr::Add {
                dst: VirtualReg(10),
                lhs: value,
                rhs: v(0),
            }),
            scalar(VirtualInstr::Add {
                dst: VirtualReg(11),
                lhs: v(10),
                rhs: v(1),
            }),
            scalar(VirtualInstr::Return(v(11))),
        ]);
        let function = compile_stream(code, 12);
        function.emit_machine_code().unwrap();
        assert_eq!(function.stack_slots, 0, "{:?}", function.instructions);
        for input in [0i64, 1, 17, i64::MIN, -91] {
            for count in [0i64, 1, 31, 63, 64, 255, 256] {
                let amount = if shared { input } else { count } as u32;
                let rotated = (input as u64).rotate_left(amount.wrapping_mul(8));
                assert_eq!(
                    eval::evaluate(&function, &[input, count]).unwrap(),
                    (rotated as i64).wrapping_add(input).wrapping_add(count)
                );
            }
        }
    }
}
