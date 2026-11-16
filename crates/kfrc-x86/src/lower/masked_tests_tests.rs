use super::*;
use crate::machine::eval;
use kfrc_body::{FunctionAbi, RecordedTypes};

fn scalar(instruction: VirtualInstr) -> LowerInstr {
    LowerInstr::Scalar(instruction)
}

fn branch(mask: i64, condition: Condition, copies: bool, shared: bool) -> Vec<LowerInstr> {
    let source = VirtualReg(0);
    let masked = VirtualReg(1);
    let tested = if copies { VirtualReg(2) } else { masked };
    let mut code = vec![
        scalar(VirtualInstr::LoadArg {
            dst: source,
            index: 0,
        }),
        scalar(VirtualInstr::And {
            dst: masked,
            lhs: VirtualValue::Reg(source),
            rhs: VirtualValue::Imm(mask),
        }),
    ];
    if copies {
        code.push(scalar(VirtualInstr::Mov {
            dst: tested,
            src: VirtualValue::Reg(masked),
        }));
    }
    code.extend([
        scalar(VirtualInstr::Test {
            lhs: VirtualValue::Reg(tested),
            rhs: VirtualValue::Imm(0),
        }),
        scalar(VirtualInstr::Jcc {
            condition,
            target: 1,
        }),
        scalar(VirtualInstr::Return(VirtualValue::Reg(if shared {
            masked
        } else {
            source
        }))),
        scalar(VirtualInstr::Label(1)),
        scalar(VirtualInstr::Return(VirtualValue::Imm(19))),
    ]);
    code
}

fn compile(code: Vec<LowerInstr>) -> MachineFunction {
    let graph = FunctionGraph::new(
        "masked_tests",
        FunctionAbi {
            args: vec![PassMode::Direct],
            ret: PassMode::Direct,
            c_variadic: false,
        },
        RecordedTypes::new(),
    );
    allocated::compile(&graph, code, 16, &[]).unwrap()
}

#[test]
fn masked_tests_preserve_all_bits_of_the_live_source() {
    // Include bit31: TEST64 with a positive imm32 would also test high bits,
    // so positive32 masks must use TEST32; negative masks require TEST64.
    for (mask, width) in [
        (3, BitWidth::W8),
        (0x80, BitWidth::W8),
        (0x100, BitWidth::W32),
        (0xffff, BitWidth::W32),
        (0x8000_0000, BitWidth::W32),
        (0xffff_ffff, BitWidth::W32),
        (-1, BitWidth::W64),
        (i64::from(i32::MIN), BitWidth::W64),
    ] {
        for condition in [Condition::Eq, Condition::NotEq] {
            for copies in [false, true] {
                let original = branch(mask, condition, copies, false);
                let before = compile(original.clone());
                let mut rewritten = original;
                assert_eq!(run(&mut rewritten), 1, "{mask:#x} {condition:?}");
                assert!(rewritten.iter().any(|i| matches!(i,
                    LowerInstr::TestMasked { width: actual, .. } if *actual == width)));
                assert!(
                    !rewritten
                        .iter()
                        .any(|i| matches!(i, LowerInstr::Scalar(VirtualInstr::And { .. })))
                );
                let after = compile(rewritten);
                // Force encoder and assembly printer through the real variant.
                after.emit_machine_code().unwrap();
                after.emit_gnu_assembly().unwrap();
                let mut values = vec![
                    0,
                    1,
                    3,
                    -1,
                    i64::MIN,
                    i64::MAX,
                    mask,
                    !mask,
                    0x80,
                    0x100,
                    0x8000_0000,
                    1i64 << 32,
                ];
                for bit in 0..64 {
                    values.push(1i64.wrapping_shl(bit));
                    values.push(!1i64.wrapping_shl(bit));
                }
                for value in values {
                    assert_eq!(
                        eval::evaluate(&before, &[value]).unwrap(),
                        eval::evaluate(&after, &[value]).unwrap(),
                        "mask {mask:#x}, input {value:#x}, {condition:?}"
                    );
                }
            }
        }
    }
}

#[test]
fn shared_results_ordered_flags_and_uncodable_masks_stay_unchanged() {
    for mut code in [
        branch(3, Condition::Eq, false, true),
        branch(3, Condition::SignedLess, false, false),
        branch(0, Condition::Eq, false, false),
        branch(1i64 << 40, Condition::Eq, false, false),
    ] {
        assert_eq!(run(&mut code), 0, "{code:?}");
    }
    // A second source definition would make using the source at the later
    // TEST observe another value; retain the captured AND result.
    let mut redefined = branch(3, Condition::Eq, false, false);
    redefined.insert(
        2,
        scalar(VirtualInstr::Mov {
            dst: VirtualReg(0),
            src: VirtualValue::Imm(7),
        }),
    );
    assert_eq!(run(&mut redefined), 0);
    // A second flags consumer can observe SF, even when the first uses ZF.
    let mut flags = branch(3, Condition::Eq, false, false);
    flags.insert(
        4,
        scalar(VirtualInstr::Jcc {
            condition: Condition::SignedLess,
            target: 1,
        }),
    );
    assert_eq!(run(&mut flags), 0);
    // Width/cast semantics belong to the producer: do not look through it.
    let mut cast = branch(3, Condition::Eq, false, false);
    cast.insert(
        2,
        LowerInstr::Extend {
            dst: VirtualReg(2),
            value: VirtualReg(1),
            width: BitWidth::W8,
            signed: false,
        },
    );
    cast[3] = scalar(VirtualInstr::Test {
        lhs: VirtualValue::Reg(VirtualReg(2)),
        rhs: VirtualValue::Imm(0),
    });
    assert_eq!(run(&mut cast), 0);
}
