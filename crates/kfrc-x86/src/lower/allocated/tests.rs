use super::*;
use crate::machine::eval;
use kfrc_body::{FunctionAbi, RecordedTypes};

fn compile_stream(instructions: Vec<LowerInstr>, count: u32) -> MachineFunction {
    let graph = FunctionGraph::new(
        "constraints",
        FunctionAbi {
            args: vec![],
            ret: PassMode::Direct,
            c_variadic: false,
        },
        RecordedTypes::new(),
    );
    compile(&graph, instructions, count, &[]).unwrap()
}
fn v(index: u32) -> VirtualValue {
    VirtualValue::Reg(VirtualReg(index))
}
fn scalar(i: VirtualInstr) -> LowerInstr {
    LowerInstr::Scalar(i)
}

#[test]
fn pressure_keeps_live_values_across_division_and_variable_shift() {
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
    for i in 2..26 {
        code.push(scalar(VirtualInstr::Add {
            dst: VirtualReg(i),
            lhs: v(0),
            rhs: VirtualValue::Imm(i as i64),
        }));
    }
    code.push(scalar(VirtualInstr::Div {
        dst: VirtualReg(26),
        lhs: v(0),
        rhs: v(1),
    }));
    code.push(scalar(VirtualInstr::Shl {
        dst: VirtualReg(27),
        lhs: v(26),
        rhs: v(1),
    }));
    let mut result = v(27);
    for i in 2..26 {
        let dst = VirtualReg(28 + i);
        code.push(scalar(VirtualInstr::Add {
            dst,
            lhs: result,
            rhs: v(i),
        }));
        result = VirtualValue::Reg(dst);
    }
    code.push(scalar(VirtualInstr::Return(result)));
    let function = compile_stream(code, 54);
    for a in [-91i64, 1, 151, 100001] {
        for b in [1i64, 2, 7, 31] {
            let expected = (2..26).fold((a / b).wrapping_shl(b as u32), |sum, i| {
                sum.wrapping_add(a + i)
            });
            assert_eq!(eval::evaluate(&function, &[a, b]).unwrap(), expected);
        }
    }
    assert!(
        function.stack_slots > 0,
        "fixture must exercise real spills"
    );
}

#[test]
fn parallel_phi_cycles_preserve_flags_and_loop_values() {
    let code = vec![
        scalar(VirtualInstr::LoadArg {
            dst: VirtualReg(0),
            index: 0,
        }),
        scalar(VirtualInstr::LoadArg {
            dst: VirtualReg(1),
            index: 1,
        }),
        scalar(VirtualInstr::LoadArg {
            dst: VirtualReg(2),
            index: 2,
        }),
        scalar(VirtualInstr::LoadArg {
            dst: VirtualReg(3),
            index: 3,
        }),
        scalar(VirtualInstr::Label(1)),
        scalar(VirtualInstr::Test {
            lhs: v(3),
            rhs: VirtualValue::Imm(0),
        }),
        scalar(VirtualInstr::Jcc {
            condition: Condition::Eq,
            target: 2,
        }),
        LowerInstr::ParallelCopy(vec![
            (VirtualReg(0), v(1)),
            (VirtualReg(1), v(2)),
            (VirtualReg(2), v(0)),
        ]),
        scalar(VirtualInstr::Sub {
            dst: VirtualReg(3),
            lhs: v(3),
            rhs: VirtualValue::Imm(1),
        }),
        scalar(VirtualInstr::Jmp(1)),
        scalar(VirtualInstr::Label(2)),
        scalar(VirtualInstr::Mul {
            dst: VirtualReg(4),
            lhs: v(0),
            rhs: VirtualValue::Imm(100),
        }),
        scalar(VirtualInstr::Mul {
            dst: VirtualReg(5),
            lhs: v(1),
            rhs: VirtualValue::Imm(10),
        }),
        scalar(VirtualInstr::Add {
            dst: VirtualReg(6),
            lhs: v(4),
            rhs: v(5),
        }),
        scalar(VirtualInstr::Add {
            dst: VirtualReg(7),
            lhs: v(6),
            rhs: v(2),
        }),
        scalar(VirtualInstr::Return(v(7))),
    ];
    let function = compile_stream(code, 8);
    for n in 0..20 {
        assert_eq!(
            eval::evaluate(&function, &[1, 2, 3, n]).unwrap(),
            [123, 231, 312][n as usize % 3]
        );
    }
}

#[test]
fn simple_values_need_no_argument_homes_or_spills() {
    let function = compile_stream(
        vec![
            scalar(VirtualInstr::LoadArg {
                dst: VirtualReg(0),
                index: 0,
            }),
            scalar(VirtualInstr::Return(v(0))),
        ],
        1,
    );
    assert_eq!(function.stack_slots, 0);
    assert!(function.callee_saved.is_empty());
    assert!(
        function.instructions.len() <= 3,
        "{:?}",
        function.instructions
    );
    assert_eq!(eval::evaluate(&function, &[37]).unwrap(), 37);
    assert_eq!(
        function.emit_machine_code().unwrap(),
        [0x48, 0x89, 0xf8, 0xc3]
    );
    let assembly = function.emit_gnu_assembly().unwrap();
    assert!(!assembly.contains("rbp"));
    assert!(!assembly.contains("leave"));
}

#[test]
fn live_across_call_prefers_preserved_register_without_spill() {
    let stream = vec![
        scalar(VirtualInstr::LoadArg {
            dst: VirtualReg(0),
            index: 0,
        }),
        scalar(VirtualInstr::Call {
            dst: VirtualReg(1),
            target: "callee".into(),
            args: vec![v(0)],
        }),
        scalar(VirtualInstr::Add {
            dst: VirtualReg(2),
            lhs: v(0),
            rhs: v(1),
        }),
        scalar(VirtualInstr::Return(v(2))),
    ];
    let function = compile_stream(stream, 3);
    assert_eq!(function.stack_slots, 0, "{:?}", function.instructions);
    assert_eq!(function.callee_saved.len(), 1);
}

#[test]
fn materializing_constants_does_not_spill_an_unrelated_live_value() {
    let mut code = vec![scalar(VirtualInstr::LoadArg {
        dst: VirtualReg(0),
        index: 0,
    })];
    let mut value = v(0);
    for i in 1..65 {
        code.push(scalar(VirtualInstr::Add {
            dst: VirtualReg(i),
            lhs: value,
            rhs: VirtualValue::Imm(i as i64),
        }));
        value = v(i);
    }
    code.push(scalar(VirtualInstr::Add {
        dst: VirtualReg(65),
        lhs: value,
        rhs: v(0),
    }));
    code.push(scalar(VirtualInstr::Return(v(65))));
    let function = compile_stream(code, 66);
    assert_eq!(function.stack_slots, 0, "{:?}", function.instructions);
    for x in [-100i64, 0, 100, i64::MAX] {
        assert_eq!(
            eval::evaluate(&function, &[x]).unwrap(),
            x.wrapping_mul(2).wrapping_add(2080)
        );
    }
}

#[test]
fn a_clobber_split_is_not_the_death_of_a_reuse_source() {
    // Five values fill the preserved bank. The sixth lives in a caller-saved
    // register up to the call; its pre-call fragment ends immediately after
    // the Add, but its value must survive both that Add and the call.
    let mut code: Vec<_> = (0..6)
        .map(|i| {
            scalar(VirtualInstr::LoadArg {
                dst: VirtualReg(i),
                index: i as usize,
            })
        })
        .collect();
    code.push(scalar(VirtualInstr::Add {
        dst: VirtualReg(6),
        lhs: v(5),
        rhs: VirtualValue::Imm(1),
    }));
    code.push(scalar(VirtualInstr::Call {
        dst: VirtualReg(7),
        target: "callee".into(),
        args: vec![v(6)],
    }));
    let mut sum = v(7);
    for i in 0..7 {
        code.push(scalar(VirtualInstr::Add {
            dst: VirtualReg(8 + i),
            lhs: sum,
            rhs: v(i),
        }));
        sum = v(8 + i);
    }
    code.push(scalar(VirtualInstr::Return(sum)));
    // The symbolic checker verifies both the old input and the new output at
    // the reuse instruction, and preservation through the call.
    let function = compile_stream(code, 15);
    assert!(
        function
            .instructions
            .iter()
            .any(|i| matches!(i, MachineInstr::Call { .. }))
    );
}

#[test]
fn mutually_exclusive_live_ranges_do_not_force_spills() {
    let mut code = vec![
        scalar(VirtualInstr::LoadArg {
            dst: VirtualReg(0),
            index: 0,
        }),
        scalar(VirtualInstr::LoadArg {
            dst: VirtualReg(1),
            index: 1,
        }),
        scalar(VirtualInstr::Test {
            lhs: v(0),
            rhs: VirtualValue::Imm(0),
        }),
        scalar(VirtualInstr::Jcc {
            condition: Condition::Eq,
            target: 1,
        }),
    ];
    // The first arm's values cross the second arm in textual layout only.
    for i in 0..10 {
        code.push(scalar(VirtualInstr::Add {
            dst: VirtualReg(2 + i),
            lhs: v(1),
            rhs: VirtualValue::Imm(i as i64),
        }));
    }
    code.push(scalar(VirtualInstr::Jmp(2)));
    code.push(scalar(VirtualInstr::Label(1)));
    for i in 0..10 {
        code.push(scalar(VirtualInstr::Sub {
            dst: VirtualReg(12 + i),
            lhs: v(1),
            rhs: VirtualValue::Imm(i as i64),
        }));
    }
    let mut sum = v(12);
    for i in 1..10 {
        code.push(scalar(VirtualInstr::Add {
            dst: VirtualReg(21 + i),
            lhs: sum,
            rhs: v(12 + i),
        }));
        sum = v(21 + i);
    }
    code.push(scalar(VirtualInstr::Return(sum)));
    code.push(scalar(VirtualInstr::Label(2)));
    sum = v(2);
    for i in 1..10 {
        code.push(scalar(VirtualInstr::Add {
            dst: VirtualReg(30 + i),
            lhs: sum,
            rhs: v(2 + i),
        }));
        sum = v(30 + i);
    }
    code.push(scalar(VirtualInstr::Return(sum)));
    let function = compile_stream(code, 40);
    assert_eq!(function.stack_slots, 0, "{:?}", function.instructions);
    for x in [-100i64, 0, 100] {
        assert_eq!(eval::evaluate(&function, &[0, x]).unwrap(), 10 * x - 45);
        assert_eq!(eval::evaluate(&function, &[1, x]).unwrap(), 10 * x + 45);
    }
}

fn binary(op: u8, dst: VirtualReg, lhs: VirtualValue, rhs: VirtualValue) -> VirtualInstr {
    match op {
        0 => VirtualInstr::Add { dst, lhs, rhs },
        1 => VirtualInstr::Sub { dst, lhs, rhs },
        2 => VirtualInstr::Mul { dst, lhs, rhs },
        3 => VirtualInstr::And { dst, lhs, rhs },
        4 => VirtualInstr::Or { dst, lhs, rhs },
        _ => VirtualInstr::Xor { dst, lhs, rhs },
    }
}

fn apply(op: u8, a: i64, b: i64) -> i64 {
    match op {
        0 => a.wrapping_add(b),
        1 => a.wrapping_sub(b),
        2 => a.wrapping_mul(b),
        3 => a & b,
        4 => a | b,
        _ => a ^ b,
    }
}

const IMMEDIATES: [i64; 7] = [
    0,
    1,
    -1,
    0x7f,
    i32::MAX as i64,
    i32::MIN as i64,
    0x1_0000_0000,
];

#[test]
fn immediate_operands_select_imm32_forms_and_keep_operand_order() {
    for op in 0..6u8 {
        for constant in IMMEDIATES {
            for constant_first in [false, true] {
                let (lhs, rhs) = if constant_first {
                    (VirtualValue::Imm(constant), v(0))
                } else {
                    (v(0), VirtualValue::Imm(constant))
                };
                let code = vec![
                    scalar(VirtualInstr::LoadArg {
                        dst: VirtualReg(0),
                        index: 0,
                    }),
                    scalar(binary(op, VirtualReg(1), lhs, rhs)),
                    scalar(VirtualInstr::Return(v(1))),
                ];
                let function = compile_stream(code, 2);
                for a in [0i64, 5, -7, i64::MAX, i64::MIN] {
                    let expected = if constant_first {
                        apply(op, constant, a)
                    } else {
                        apply(op, a, constant)
                    };
                    assert_eq!(
                        eval::evaluate(&function, &[a]).unwrap(),
                        expected,
                        "op {op} constant {constant} first {constant_first} a {a}"
                    );
                }
                // Only Sub with a leading constant and non-imm32 constants need
                // the constant in a register.
                let in_register = function.instructions.iter().any(|i| match i {
                    MachineInstr::MovImm { value, .. } => *value == constant,
                    // The zero idiom replaces `MovImm 0`.
                    MachineInstr::Xor {
                        dst,
                        src: crate::machine::x86::Operand::Reg(src),
                    } => constant == 0 && dst == src,
                    _ => false,
                });
                let foldable = i32::try_from(constant).is_ok() && !(op == 1 && constant_first);
                assert_eq!(in_register, !foldable, "op {op} constant {constant}");
            }
        }
    }
}

#[test]
fn immediate_comparisons_preserve_every_condition_at_boundaries() {
    use Condition::*;
    let conditions = [
        (Eq, (|a, b| a == b) as fn(i64, i64) -> bool),
        (NotEq, |a, b| a != b),
        (SignedLess, |a, b| a < b),
        (SignedLessEq, |a, b| a <= b),
        (SignedGreater, |a, b| a > b),
        (SignedGreaterEq, |a, b| a >= b),
        (UnsignedBelow, |a, b| (a as u64) < (b as u64)),
        (UnsignedBelowEq, |a, b| (a as u64) <= (b as u64)),
        (UnsignedAbove, |a, b| (a as u64) > (b as u64)),
        (UnsignedAboveEq, |a, b| (a as u64) >= (b as u64)),
    ];
    for (condition, holds) in conditions {
        for constant in IMMEDIATES {
            let code = vec![
                scalar(VirtualInstr::LoadArg {
                    dst: VirtualReg(0),
                    index: 0,
                }),
                scalar(VirtualInstr::Compare {
                    dst: VirtualReg(1),
                    condition,
                    lhs: v(0),
                    rhs: VirtualValue::Imm(constant),
                }),
                scalar(VirtualInstr::Return(v(1))),
            ];
            let function = compile_stream(code, 2);
            for a in [
                constant,
                constant.wrapping_sub(1),
                constant.wrapping_add(1),
                -1,
                0,
            ] {
                assert_eq!(
                    eval::evaluate(&function, &[a]).unwrap(),
                    i64::from(holds(a, constant)),
                    "{condition:?} {a} vs {constant}"
                );
            }
        }
    }
}

fn phi_undefined_on_skip_path(count: u32, call: bool) -> Vec<LowerInstr> {
    // v1 decides the path. Phi values v(10..) are defined only on the
    // fallthrough arm; the skip arm has an empty copy set, as for Undefined.
    let mut code = vec![scalar(VirtualInstr::LoadArg {
        dst: VirtualReg(0),
        index: 0,
    })];
    if call {
        code.push(scalar(VirtualInstr::Call {
            dst: VirtualReg(1),
            target: "callee".into(),
            args: vec![v(0)],
        }));
    } else {
        code.push(scalar(VirtualInstr::Mov {
            dst: VirtualReg(1),
            src: v(0),
        }));
    }
    code.push(scalar(VirtualInstr::Test {
        lhs: v(1),
        rhs: VirtualValue::Imm(0),
    }));
    code.push(scalar(VirtualInstr::Jcc {
        condition: Condition::Eq,
        target: 1,
    }));
    for i in 0..count {
        code.push(scalar(VirtualInstr::Add {
            dst: VirtualReg(100 + i),
            lhs: v(0),
            rhs: VirtualValue::Imm(i as i64 + 1),
        }));
    }
    code.push(LowerInstr::ParallelCopy(
        (0..count)
            .map(|i| (VirtualReg(10 + i), v(100 + i)))
            .collect(),
    ));
    code.push(scalar(VirtualInstr::Jmp(2)));
    code.push(scalar(VirtualInstr::Label(1)));
    code.push(LowerInstr::ParallelCopy(Vec::new()));
    code.push(scalar(VirtualInstr::Jmp(2)));
    code.push(scalar(VirtualInstr::Label(2)));
    code.push(scalar(VirtualInstr::Test {
        lhs: v(1),
        rhs: VirtualValue::Imm(0),
    }));
    code.push(scalar(VirtualInstr::Jcc {
        condition: Condition::Eq,
        target: 3,
    }));
    let mut sum = v(10);
    for i in 1..count {
        code.push(scalar(VirtualInstr::Add {
            dst: VirtualReg(200 + i),
            lhs: sum,
            rhs: v(10 + i),
        }));
        sum = v(200 + i);
    }
    code.push(scalar(VirtualInstr::Return(sum)));
    code.push(scalar(VirtualInstr::Label(3)));
    code.push(scalar(VirtualInstr::Return(VirtualValue::Imm(7))));
    code
}

#[test]
fn phi_values_undefined_on_one_path_are_not_live_before_their_copies() {
    // Before reaching definitions limited liveness, the eight Phi values
    // were live from the entry, crossed the call and spilled garbage.
    let function = compile_stream(phi_undefined_on_skip_path(8, true), 300);
    let call = function
        .instructions
        .iter()
        .position(|i| matches!(i, MachineInstr::Call { .. }))
        .unwrap();
    assert!(
        function.instructions[..call]
            .iter()
            .all(|i| !matches!(i, MachineInstr::Store { .. })),
        "{:?}",
        function.instructions
    );
    assert_eq!(function.stack_slots, 0, "{:?}", function.instructions);
    let function = compile_stream(phi_undefined_on_skip_path(8, false), 300);
    for x in [-5i64, 0, 3, 1 << 40] {
        let expected = if x == 0 {
            7
        } else {
            (1..=8).map(|i| x + i).sum::<i64>()
        };
        assert_eq!(eval::evaluate(&function, &[x]).unwrap(), expected);
    }
}

#[test]
fn uses_of_values_undefined_on_every_path_read_zero() {
    // v2 is a Phi defined only on the fallthrough arm. On the skip arm a
    // second Phi copies it and an Add reads it: both read no definition.
    let code = vec![
        scalar(VirtualInstr::LoadArg {
            dst: VirtualReg(0),
            index: 0,
        }),
        scalar(VirtualInstr::Test {
            lhs: v(0),
            rhs: VirtualValue::Imm(0),
        }),
        scalar(VirtualInstr::Jcc {
            condition: Condition::Eq,
            target: 1,
        }),
        scalar(VirtualInstr::Add {
            dst: VirtualReg(1),
            lhs: v(0),
            rhs: VirtualValue::Imm(1),
        }),
        LowerInstr::ParallelCopy(vec![(VirtualReg(2), v(1)), (VirtualReg(3), v(0))]),
        scalar(VirtualInstr::Jmp(2)),
        scalar(VirtualInstr::Label(1)),
        scalar(VirtualInstr::Add {
            dst: VirtualReg(4),
            lhs: v(2),
            rhs: VirtualValue::Imm(9),
        }),
        LowerInstr::ParallelCopy(vec![(VirtualReg(3), v(4)), (VirtualReg(5), v(2))]),
        scalar(VirtualInstr::Jmp(2)),
        scalar(VirtualInstr::Label(2)),
        scalar(VirtualInstr::Add {
            dst: VirtualReg(6),
            lhs: v(3),
            rhs: VirtualValue::Imm(100),
        }),
        scalar(VirtualInstr::Return(v(6))),
    ];
    let instructions = constraints::select(code.clone());
    let parallel: Vec<_> = instructions
        .iter()
        .filter_map(|i| match &i.operation {
            Operation::Parallel(copies) => Some(copies.len()),
            _ => None,
        })
        .collect();
    assert_eq!(parallel, [2, 1], "the copy of undefined v2 is dropped");
    let function = compile_stream(code, 7);
    for x in [-5i64, 0, 3] {
        let expected = if x == 0 { 109 } else { x + 100 };
        assert_eq!(eval::evaluate(&function, &[x]).unwrap(), expected);
    }
}

fn repeated_pressure_rounds(rounds: u32, read_early_last: bool) -> (Vec<LowerInstr>, u32) {
    // v0 is read once per round. Each round keeps fifteen fresh values live
    // together, so v0 is evicted and reloaded around every round. When
    // `read_early_last` is set, the last round reads v0 first in its sum, so
    // v0 stays in a register there while the round's own values spill.
    let mut code = vec![scalar(VirtualInstr::LoadArg {
        dst: VirtualReg(0),
        index: 0,
    })];
    let mut next = 1;
    let mut fresh = || {
        next += 1;
        VirtualReg(next - 1)
    };
    let mut total = v(0);
    for round in 0..rounds {
        let values: Vec<_> = (0..15)
            .map(|i| {
                let dst = fresh();
                code.push(scalar(VirtualInstr::Mul {
                    dst,
                    lhs: total,
                    rhs: VirtualValue::Imm(i64::from(round * 16 + i + 3)),
                }));
                dst
            })
            .collect();
        // v0 is read last, so it has the furthest next use in every round.
        let mut order: Vec<_> = values[1..].to_vec();
        if read_early_last && round + 1 == rounds {
            order.insert(0, VirtualReg(0));
        } else {
            order.push(VirtualReg(0));
        }
        let mut sum = VirtualValue::Reg(values[0]);
        for &value in &order {
            let dst = fresh();
            code.push(scalar(VirtualInstr::Add {
                dst,
                lhs: sum,
                rhs: VirtualValue::Reg(value),
            }));
            sum = VirtualValue::Reg(dst);
        }
        total = sum;
    }
    let dst = fresh();
    code.push(scalar(VirtualInstr::Add {
        dst,
        lhs: total,
        rhs: v(0),
    }));
    code.push(scalar(VirtualInstr::Return(VirtualValue::Reg(dst))));
    (code, next)
}

#[test]
fn a_value_spilled_in_every_round_is_stored_once_at_its_definition() {
    let (code, count) = repeated_pressure_rounds(4, false);
    let function = compile_stream(code, count);
    let stores = function
        .instructions
        .iter()
        .filter(|i| matches!(i, MachineInstr::Store { .. }))
        .count();
    let loads = function
        .instructions
        .iter()
        .filter(|i| matches!(i, MachineInstr::Load { .. }))
        .count();
    assert!(loads >= 4, "fixture must reload v0 per round: {function:?}");
    assert!(stores < loads, "{:?}", function.instructions);
    for x in [-3i64, 0, 2, 1 << 20] {
        let mut total = x;
        for round in 0..4i64 {
            let mut sum = x;
            for i in 0..15 {
                sum = sum.wrapping_add(total.wrapping_mul(round * 16 + i + 3));
            }
            total = sum;
        }
        assert_eq!(
            eval::evaluate(&function, &[x]).unwrap(),
            total.wrapping_add(x)
        );
    }
}

#[test]
fn a_memory_slot_is_reserved_while_its_value_lives_in_a_register() {
    // v0 is stored at its definition. After its last stack fragment it stays
    // live in a register while the final round's values spill; none of them
    // may reuse v0's slot, which every later position still relies on.
    for rounds in 2..5 {
        let (code, count) = repeated_pressure_rounds(rounds, true);
        let function = compile_stream(code, count);
        for x in [-3i64, 0, 2, 1 << 20] {
            let mut total = x;
            for round in 0..i64::from(rounds) {
                let mut sum = x;
                for i in 0..15 {
                    sum = sum.wrapping_add(total.wrapping_mul(round * 16 + i + 3));
                }
                total = sum;
            }
            assert_eq!(
                eval::evaluate(&function, &[x]).unwrap(),
                total.wrapping_add(x)
            );
        }
    }
}

#[test]
fn a_value_dying_as_a_call_argument_is_computed_in_its_argument_register() {
    let stream = vec![
        scalar(VirtualInstr::LoadArg {
            dst: VirtualReg(0),
            index: 0,
        }),
        scalar(VirtualInstr::Add {
            dst: VirtualReg(1),
            lhs: v(0),
            rhs: VirtualValue::Imm(5),
        }),
        scalar(VirtualInstr::Add {
            dst: VirtualReg(2),
            lhs: v(0),
            rhs: VirtualValue::Imm(7),
        }),
        scalar(VirtualInstr::Call {
            dst: VirtualReg(3),
            target: "callee".into(),
            args: vec![v(2), v(1)],
        }),
        scalar(VirtualInstr::Add {
            dst: VirtualReg(4),
            lhs: v(3),
            rhs: v(0),
        }),
        scalar(VirtualInstr::Return(v(4))),
    ];
    let function = compile_stream(stream, 5);
    let call = function
        .instructions
        .iter()
        .position(|i| matches!(i, MachineInstr::Call { .. }))
        .unwrap();
    // v1 and v2 are computed directly in rsi and rdi; only v0 is copied once
    // into a preserved register, and the call result is added in place.
    assert!(
        !matches!(function.instructions[call - 1], MachineInstr::Mov { .. }),
        "{:?}",
        function.instructions
    );
    let moves = function
        .instructions
        .iter()
        .filter(|i| matches!(i, MachineInstr::Mov { .. }))
        .count();
    assert!(moves <= 3, "{:?}", function.instructions);
}

#[test]
fn a_fixed_result_takes_the_register_of_its_dying_fixed_input() {
    // Each quotient dies as the next division's rax input, and the next
    // quotient is written to rax. No value needs to leave rax in between.
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
    let mut quotient = v(0);
    for i in 2..6 {
        code.push(scalar(VirtualInstr::Div {
            dst: VirtualReg(i),
            lhs: quotient,
            rhs: v(1),
        }));
        quotient = v(i);
    }
    code.push(scalar(VirtualInstr::Return(quotient)));
    let function = compile_stream(code, 6);
    let moves = function
        .instructions
        .iter()
        .filter(|i| matches!(i, MachineInstr::Mov { .. }))
        .count();
    // rdi -> rax for the dividend and rsi -> a register outside rax/rdx.
    assert!(moves <= 2, "{:?}", function.instructions);
    for (a, b) in [(1_000_000i64, 3i64), (-77_777, 2), (5, 7)] {
        assert_eq!(
            eval::evaluate(&function, &[a, b]).unwrap(),
            a / b / b / b / b
        );
    }
}

/// Fifteen products of `base`, summed: all are live together before the sum.
fn pressure_round(code: &mut Vec<LowerInstr>, base: VirtualValue, first: u32) -> VirtualValue {
    for i in 0..15 {
        code.push(scalar(VirtualInstr::Mul {
            dst: VirtualReg(first + i),
            lhs: base,
            rhs: VirtualValue::Imm(i64::from(i) + 3),
        }));
    }
    let mut sum = v(first);
    for i in 1..15 {
        code.push(scalar(VirtualInstr::Add {
            dst: VirtualReg(first + 15 + i),
            lhs: sum,
            rhs: v(first + i),
        }));
        sum = v(first + 15 + i);
    }
    sum
}

#[test]
fn a_value_spilled_on_every_entry_edge_stays_in_its_slot_until_used() {
    // v0 spills under the pressure of the first round. The skip arm returns,
    // leaving a lifetime hole before the join, where a second round runs
    // before v0's only read. Reloading v0 on the jump into the join would
    // only spill it again, so it stays in its slot until that read.
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
    let first = pressure_round(&mut code, v(1), 10);
    code.push(scalar(VirtualInstr::Test {
        lhs: first,
        rhs: VirtualValue::Imm(0),
    }));
    code.push(scalar(VirtualInstr::Jcc {
        condition: Condition::Eq,
        target: 1,
    }));
    code.push(scalar(VirtualInstr::Jmp(2)));
    code.push(scalar(VirtualInstr::Label(1)));
    code.push(scalar(VirtualInstr::Return(VirtualValue::Imm(-1))));
    code.push(scalar(VirtualInstr::Label(2)));
    let second = pressure_round(&mut code, first, 50);
    code.push(scalar(VirtualInstr::Add {
        dst: VirtualReg(90),
        lhs: second,
        rhs: v(0),
    }));
    code.push(scalar(VirtualInstr::Return(v(90))));
    let function = compile_stream(code, 91);
    let jump = function
        .instructions
        .iter()
        .position(|i| matches!(i, MachineInstr::Jmp(2)))
        .unwrap();
    assert!(
        !matches!(function.instructions[jump - 1], MachineInstr::Load { .. }),
        "{:?}",
        function.instructions
    );
    let products = (3..18).sum::<i64>();
    for (a, b) in [(5i64, 7i64), (-9, 2), (1, 1 << 20)] {
        let first = b.wrapping_mul(products);
        let expected = if first == 0 {
            -1
        } else {
            first.wrapping_mul(products).wrapping_add(a)
        };
        assert_eq!(eval::evaluate(&function, &[a, b]).unwrap(), expected);
    }
}

#[test]
fn immediate_left_comparisons_mirror_their_condition() {
    let conditions = [
        Condition::Eq,
        Condition::NotEq,
        Condition::SignedLess,
        Condition::SignedLessEq,
        Condition::SignedGreater,
        Condition::SignedGreaterEq,
        Condition::UnsignedBelow,
        Condition::UnsignedBelowEq,
        Condition::UnsignedAbove,
        Condition::UnsignedAboveEq,
    ];
    let holds = |condition: Condition, lhs: i64, rhs: i64| match condition {
        Condition::Eq => lhs == rhs,
        Condition::NotEq => lhs != rhs,
        Condition::SignedLess => lhs < rhs,
        Condition::SignedLessEq => lhs <= rhs,
        Condition::SignedGreater => lhs > rhs,
        Condition::SignedGreaterEq => lhs >= rhs,
        Condition::UnsignedBelow => (lhs as u64) < rhs as u64,
        Condition::UnsignedBelowEq => (lhs as u64) <= rhs as u64,
        Condition::UnsignedAbove => (lhs as u64) > rhs as u64,
        Condition::UnsignedAboveEq => (lhs as u64) >= rhs as u64,
    };
    for condition in conditions {
        // Compare 5 with x into a Boolean, and branch on 5 against x; the
        // result is 10 * branch + Boolean.
        let code = vec![
            scalar(VirtualInstr::LoadArg {
                dst: VirtualReg(0),
                index: 0,
            }),
            scalar(VirtualInstr::Compare {
                dst: VirtualReg(1),
                condition,
                lhs: VirtualValue::Imm(5),
                rhs: v(0),
            }),
            scalar(VirtualInstr::Test {
                lhs: VirtualValue::Imm(5),
                rhs: v(0),
            }),
            scalar(VirtualInstr::Jcc {
                condition,
                target: 1,
            }),
            scalar(VirtualInstr::Return(v(1))),
            scalar(VirtualInstr::Label(1)),
            scalar(VirtualInstr::Add {
                dst: VirtualReg(2),
                lhs: v(1),
                rhs: VirtualValue::Imm(10),
            }),
            scalar(VirtualInstr::Return(v(2))),
        ];
        let function = compile_stream(code, 3);
        assert!(
            function.instructions.iter().all(|i| !matches!(
                i,
                MachineInstr::Cmp {
                    rhs: MachineOperand::Reg(_),
                    ..
                }
            )),
            "{condition:?}: {:?}",
            function.instructions
        );
        for x in [-7i64, 4, 5, 6, i64::MAX, i64::MIN] {
            let expected = i64::from(holds(condition, 5, x)) * 11;
            assert_eq!(
                eval::evaluate(&function, &[x]).unwrap(),
                expected,
                "{condition:?} {x}"
            );
        }
    }
}

#[test]
fn an_immediate_add_with_a_live_source_uses_a_three_address_lea() {
    let code = vec![
        scalar(VirtualInstr::LoadArg {
            dst: VirtualReg(0),
            index: 0,
        }),
        scalar(VirtualInstr::Add {
            dst: VirtualReg(1),
            lhs: v(0),
            rhs: VirtualValue::Imm(5),
        }),
        scalar(VirtualInstr::Sub {
            dst: VirtualReg(2),
            lhs: v(0),
            rhs: VirtualValue::Imm(7),
        }),
        scalar(VirtualInstr::Sub {
            dst: VirtualReg(3),
            lhs: v(0),
            rhs: VirtualValue::Imm(i64::from(i32::MIN)),
        }),
        scalar(VirtualInstr::Mul {
            dst: VirtualReg(4),
            lhs: v(1),
            rhs: v(2),
        }),
        scalar(VirtualInstr::Xor {
            dst: VirtualReg(5),
            lhs: v(4),
            rhs: v(3),
        }),
        scalar(VirtualInstr::Return(v(5))),
    ];
    let function = compile_stream(code, 6);
    let offsets: Vec<_> = function
        .instructions
        .iter()
        .filter_map(|i| match i {
            MachineInstr::OffsetAddress { offset, .. } => Some(*offset),
            _ => None,
        })
        .collect();
    // v0 stays live in place; i32::MIN has no negated displacement.
    assert_eq!(offsets, [5, -7], "{:?}", function.instructions);
    for x in [-3i64, 0, 11, i64::MAX] {
        let expected = (x.wrapping_add(5).wrapping_mul(x.wrapping_sub(7)))
            ^ x.wrapping_sub(i64::from(i32::MIN));
        assert_eq!(eval::evaluate(&function, &[x]).unwrap(), expected);
    }
}

#[test]
fn a_taken_edge_with_copies_before_a_plain_jump_needs_no_edge_block() {
    // Label 1 is reached by the taken branch and later by the loop back
    // edge; v0 and v1 swap homes around the loop, so the taken edge from
    // the entry needs copies. The fallthrough only jumps to label 2.
    let code = vec![
        scalar(VirtualInstr::LoadArg {
            dst: VirtualReg(0),
            index: 0,
        }),
        scalar(VirtualInstr::LoadArg {
            dst: VirtualReg(1),
            index: 1,
        }),
        scalar(VirtualInstr::Test {
            lhs: v(1),
            rhs: VirtualValue::Imm(0),
        }),
        scalar(VirtualInstr::Jcc {
            condition: Condition::NotEq,
            target: 1,
        }),
        scalar(VirtualInstr::Jmp(2)),
        scalar(VirtualInstr::Label(1)),
        scalar(VirtualInstr::Sub {
            dst: VirtualReg(2),
            lhs: v(1),
            rhs: VirtualValue::Imm(1),
        }),
        scalar(VirtualInstr::Add {
            dst: VirtualReg(3),
            lhs: v(0),
            rhs: VirtualValue::Imm(3),
        }),
        LowerInstr::ParallelCopy(vec![(VirtualReg(0), v(2)), (VirtualReg(1), v(3))]),
        scalar(VirtualInstr::Test {
            lhs: v(0),
            rhs: VirtualValue::Imm(0),
        }),
        scalar(VirtualInstr::Jcc {
            condition: Condition::NotEq,
            target: 1,
        }),
        scalar(VirtualInstr::Jmp(2)),
        scalar(VirtualInstr::Label(2)),
        scalar(VirtualInstr::Return(v(0))),
    ];
    let function = compile_stream(code, 4);
    // Edge blocks would introduce labels beyond the stream's own two.
    assert!(
        function
            .instructions
            .iter()
            .all(|i| !matches!(i, MachineInstr::Label(label) if *label > 2)),
        "{:?}",
        function.instructions
    );
    // The back edge's copies run inline between the inverted branch and the
    // jump to the loop header.
    let branch = function
        .instructions
        .iter()
        .position(|i| {
            matches!(
                i,
                MachineInstr::Jcc {
                    condition: Condition::Eq,
                    target: 2
                }
            )
        })
        .unwrap();
    let rest = &function.instructions[branch + 1..];
    let copies = rest
        .iter()
        .take_while(|i| matches!(i, MachineInstr::Mov { .. }))
        .count();
    let inline = copies > 0 && rest[copies] == MachineInstr::Jmp(1);
    assert!(inline, "{:?}", function.instructions);
    for (a, b, expected) in [(7i64, 0i64, 7i64), (7, 1, 0), (-2, 2, 0)] {
        assert_eq!(eval::evaluate(&function, &[a, b]).unwrap(), expected);
    }
}

#[test]
fn constant_stores_use_immediates_except_wide_zeros() {
    let mut code = vec![scalar(VirtualInstr::LoadArg {
        dst: VirtualReg(0),
        index: 0,
    })];
    for (offset, value, bytes) in [
        (0, 0, 1),
        (1, 0x1234, 2),
        (4, 7, 4),
        (8, -9, 8),
        (16, 0, 8),
        (24, 0, 4),
        (32, 1 << 40, 8),
    ] {
        code.push(LowerInstr::Store {
            address: v(0),
            offset,
            src: VirtualValue::Imm(value),
            bytes,
        });
    }
    code.push(scalar(VirtualInstr::Return(VirtualValue::Imm(0))));
    let function = compile_stream(code, 1);
    let immediates: Vec<_> = function
        .instructions
        .iter()
        .filter_map(|i| match i {
            MachineInstr::StoreImmediate {
                offset,
                value,
                bytes,
                ..
            } => Some((*offset, *value, *bytes)),
            _ => None,
        })
        .collect();
    assert_eq!(
        immediates,
        [(0, 0, 1), (1, 0x1234, 2), (4, 7, 4), (8, -9, 8)],
        "{:?}",
        function.instructions
    );
    let registers = function
        .instructions
        .iter()
        .filter(|i| matches!(i, MachineInstr::StoreIndirect { .. }))
        .count();
    assert_eq!(registers, 3, "{:?}", function.instructions);
}

fn counting_loop_with_cold_exit() -> Vec<LowerInstr> {
    // v0 = n, v1 = step. Header phis: v2 = i, v3 = total, v4 = parity.
    // The exit test leaves to a cold block placed between the body and the
    // latch, where unrelated values occupy registers; both body arms join
    // at the latch, whose Phi copies feed the back edge.
    let mut code = vec![
        scalar(VirtualInstr::LoadArg {
            dst: VirtualReg(0),
            index: 0,
        }),
        scalar(VirtualInstr::LoadArg {
            dst: VirtualReg(1),
            index: 1,
        }),
        LowerInstr::ParallelCopy(vec![
            (VirtualReg(2), VirtualValue::Imm(0)),
            (VirtualReg(3), VirtualValue::Imm(0)),
            (VirtualReg(4), VirtualValue::Imm(0)),
        ]),
        scalar(VirtualInstr::Jmp(1)),
        scalar(VirtualInstr::Label(1)),
        scalar(VirtualInstr::Test {
            lhs: v(2),
            rhs: v(0),
        }),
        scalar(VirtualInstr::Jcc {
            condition: Condition::Eq,
            target: 3,
        }),
        scalar(VirtualInstr::Test {
            lhs: v(4),
            rhs: VirtualValue::Imm(0),
        }),
        scalar(VirtualInstr::Jcc {
            condition: Condition::Eq,
            target: 2,
        }),
        scalar(VirtualInstr::Add {
            dst: VirtualReg(5),
            lhs: v(3),
            rhs: v(1),
        }),
        LowerInstr::ParallelCopy(vec![
            (VirtualReg(6), v(5)),
            (VirtualReg(7), VirtualValue::Imm(0)),
        ]),
        scalar(VirtualInstr::Jmp(4)),
        scalar(VirtualInstr::Label(2)),
        scalar(VirtualInstr::Sub {
            dst: VirtualReg(8),
            lhs: v(3),
            rhs: VirtualValue::Imm(1),
        }),
        LowerInstr::ParallelCopy(vec![
            (VirtualReg(6), v(8)),
            (VirtualReg(7), VirtualValue::Imm(1)),
        ]),
        scalar(VirtualInstr::Jmp(4)),
        scalar(VirtualInstr::Label(3)),
    ];
    // The cold exit: twelve values derived from the total, then summed.
    for i in 0..12 {
        code.push(scalar(VirtualInstr::Mul {
            dst: VirtualReg(20 + i),
            lhs: v(3),
            rhs: VirtualValue::Imm(i64::from(i) + 2),
        }));
    }
    let mut sum = v(20);
    for i in 1..12 {
        code.push(scalar(VirtualInstr::Add {
            dst: VirtualReg(40 + i),
            lhs: sum,
            rhs: v(20 + i),
        }));
        sum = v(40 + i);
    }
    code.push(scalar(VirtualInstr::Return(sum)));
    code.push(scalar(VirtualInstr::Label(4)));
    code.push(scalar(VirtualInstr::Add {
        dst: VirtualReg(9),
        lhs: v(2),
        rhs: VirtualValue::Imm(1),
    }));
    code.push(LowerInstr::ParallelCopy(vec![
        (VirtualReg(2), v(9)),
        (VirtualReg(3), v(6)),
        (VirtualReg(4), v(7)),
    ]));
    code.push(scalar(VirtualInstr::Jmp(1)));
    code
}

#[test]
fn loop_carried_values_reach_the_back_edge_in_their_header_registers() {
    let function = compile_stream(counting_loop_with_cold_exit(), 60);
    // The latch increments the counter in place: the counter's range ends
    // at its last read, so the new count takes its register and the back
    // edge needs no copy.
    let latch = function
        .instructions
        .iter()
        .position(|i| *i == MachineInstr::Label(4))
        .unwrap();
    assert!(
        matches!(
            function.instructions[latch + 1..],
            [MachineInstr::Add { .. }, MachineInstr::Jmp(1)]
        ),
        "{:?}",
        function.instructions
    );
    for (n, step) in [(0i64, 5i64), (1, 5), (4, 3), (7, -2)] {
        let (mut total, mut parity) = (0i64, 0i64);
        for _ in 0..n {
            if parity == 0 {
                total -= 1;
                parity = 1;
            } else {
                total += step;
                parity = 0;
            }
        }
        let expected = (2..14).map(|k| total.wrapping_mul(k)).sum::<i64>();
        assert_eq!(eval::evaluate(&function, &[n, step]).unwrap(), expected);
    }
}
