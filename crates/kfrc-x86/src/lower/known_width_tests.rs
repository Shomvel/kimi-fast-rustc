use super::*;
use crate::machine::eval;
use kfrc_body::{FunctionAbi, RecordedTypes};

fn reg(n: u32) -> VirtualReg {
    VirtualReg(n)
}
fn v(n: u32) -> VirtualValue {
    VirtualValue::Reg(reg(n))
}
fn imm(n: i64) -> VirtualValue {
    VirtualValue::Imm(n)
}
fn scalar(i: VirtualInstr) -> LowerInstr {
    LowerInstr::Scalar(i)
}
fn argument(dst: u32, index: usize) -> LowerInstr {
    scalar(VirtualInstr::LoadArg {
        dst: reg(dst),
        index,
    })
}
fn and(dst: u32, lhs: VirtualValue, mask: i64) -> LowerInstr {
    scalar(VirtualInstr::And {
        dst: reg(dst),
        lhs,
        rhs: imm(mask),
    })
}
fn extend(dst: u32, value: u32, width: BitWidth, signed: bool) -> LowerInstr {
    LowerInstr::Extend {
        dst: reg(dst),
        value: reg(value),
        width,
        signed,
    }
}
fn ret(value: VirtualValue) -> LowerInstr {
    scalar(VirtualInstr::Return(value))
}

fn compile(code: Vec<LowerInstr>) -> MachineFunction {
    let graph = FunctionGraph::new(
        "known_width",
        FunctionAbi {
            args: vec![PassMode::Direct, PassMode::Direct],
            ret: PassMode::Direct,
            c_variadic: false,
        },
        RecordedTypes::new(),
    );
    allocated::compile(&graph, code, 32, &[]).unwrap()
}

const INPUTS: [i64; 10] = [
    0,
    1,
    -1,
    0x7f,
    0x80,
    0xff,
    0x100,
    0x8000,
    i64::MIN,
    0x0123_4567_89ab_cdef,
];

/// Runs the pass and checks it against the original stream on every input.
fn optimized(code: Vec<LowerInstr>) -> (Vec<LowerInstr>, Stats) {
    let original = compile(code.clone());
    let mut rewritten = code;
    let stats = run(&mut rewritten);
    let function = compile(rewritten.clone());
    for first in INPUTS {
        for second in [0, 3, -2] {
            assert_eq!(
                eval::evaluate(&function, &[first, second]).unwrap(),
                eval::evaluate(&original, &[first, second]).unwrap(),
                "inputs {first} {second}: {rewritten:?}"
            );
        }
    }
    (rewritten, stats)
}

fn count(code: &[LowerInstr], wanted: fn(&LowerInstr) -> bool) -> usize {
    code.iter()
        .filter(|instruction| wanted(instruction))
        .count()
}
fn extends(code: &[LowerInstr]) -> usize {
    count(code, |i| matches!(i, LowerInstr::Extend { .. }))
}
fn masks(code: &[LowerInstr]) -> usize {
    count(code, |i| {
        matches!(i, LowerInstr::Scalar(VirtualInstr::And { .. }))
    })
}

#[test]
fn extensions_of_masked_compared_and_loaded_values_become_moves() {
    for (mask, width, signed) in [
        (0x1f, BitWidth::W8, false),
        (0x7f, BitWidth::W8, true),
        (0xff, BitWidth::W8, false),
        (0xffff, BitWidth::W32, true),
        (0xff, BitWidth::W16, true),
    ] {
        let (code, stats) = optimized(vec![
            argument(1, 0),
            and(2, v(1), mask),
            extend(3, 2, width, signed),
            ret(v(3)),
        ]);
        assert_eq!(stats.extensions_removed, 1, "{mask:#x} {width:?} {signed}");
        assert_eq!(extends(&code), 0);
    }
    let (code, stats) = optimized(vec![
        argument(1, 0),
        argument(2, 1),
        scalar(VirtualInstr::Compare {
            dst: reg(3),
            condition: Condition::UnsignedBelow,
            lhs: v(1),
            rhs: v(2),
        }),
        extend(4, 3, BitWidth::W8, true),
        ret(v(4)),
    ]);
    assert_eq!((stats.extensions_removed, extends(&code)), (1, 0));
}

#[test]
fn masks_of_known_narrow_values_and_constants_become_moves() {
    let (code, stats) = optimized(vec![
        argument(1, 0),
        and(2, v(1), 0xff),
        and(3, v(2), 0xffff),
        and(4, imm(0x1234), 0xff),
        scalar(VirtualInstr::Add {
            dst: reg(5),
            lhs: v(3),
            rhs: v(4),
        }),
        ret(v(5)),
    ]);
    assert_eq!(stats.masks_removed, 2);
    assert_eq!(masks(&code), 1);
}

#[test]
fn a_mask_read_only_by_a_narrower_extension_is_bypassed() {
    let (code, stats) = optimized(vec![
        argument(1, 0),
        and(2, v(1), 0xff),
        extend(3, 2, BitWidth::W8, true),
        ret(v(3)),
    ]);
    assert_eq!(stats.masks_bypassed, 1);
    assert_eq!((masks(&code), extends(&code)), (0, 1));
}

#[test]
fn unknown_wide_shared_and_redefined_inputs_keep_their_canonicalization() {
    // Unknown argument bits.
    let (code, stats) = optimized(vec![
        argument(1, 0),
        extend(2, 1, BitWidth::W8, false),
        ret(v(2)),
    ]);
    assert_eq!((stats, extends(&code)), (Stats::default(), 1));
    // A full byte is not a sign-extended byte.
    let (code, stats) = optimized(vec![
        argument(1, 0),
        and(2, v(1), 0xff),
        scalar(VirtualInstr::Add {
            dst: reg(4),
            lhs: v(2),
            rhs: imm(1),
        }),
        extend(3, 2, BitWidth::W8, true),
        scalar(VirtualInstr::Add {
            dst: reg(5),
            lhs: v(3),
            rhs: v(4),
        }),
        ret(v(5)),
    ]);
    assert_eq!(
        (stats, extends(&code), masks(&code)),
        (Stats::default(), 1, 1)
    );
    // A mask narrower than the extension changes the extended bits.
    let (code, stats) = optimized(vec![
        argument(1, 0),
        and(2, v(1), 0xff),
        extend(3, 2, BitWidth::W16, true),
        ret(v(3)),
    ]);
    assert_eq!(stats.masks_bypassed, 0);
    assert_eq!(stats.extensions_removed, 1);
    assert_eq!(masks(&code), 1);
    // Non-contiguous masks and registers with several definitions carry no width.
    let (code, stats) = optimized(vec![
        argument(1, 0),
        and(2, v(1), 0xf0),
        and(3, v(2), 0x0f),
        LowerInstr::ParallelCopy(vec![(reg(4), v(3))]),
        LowerInstr::ParallelCopy(vec![(reg(4), v(1))]),
        extend(5, 4, BitWidth::W8, false),
        ret(v(5)),
    ]);
    assert_eq!(stats.extensions_removed, 0);
    assert_eq!(extends(&code), 1);
    assert_eq!(masks(&code), 2);
}

/// `if low_byte(x) != 0 { 9 } else { 7 }` through a byte extension or mask.
fn byte_branch(mask: bool, condition: Condition, shared: bool) -> Vec<LowerInstr> {
    let mut code = vec![argument(1, 0)];
    code.push(if mask {
        and(2, v(1), 0xff)
    } else {
        extend(2, 1, BitWidth::W8, false)
    });
    code.extend([
        scalar(VirtualInstr::Test {
            lhs: v(2),
            rhs: imm(0),
        }),
        scalar(VirtualInstr::Jcc {
            condition,
            target: 2,
        }),
        ret(if shared { v(2) } else { imm(7) }),
        scalar(VirtualInstr::Label(2)),
        ret(imm(9)),
    ]);
    code
}

#[test]
fn zero_tests_of_byte_extensions_and_masks_read_the_low_byte() {
    for mask in [false, true] {
        for condition in [Condition::NotEq, Condition::Eq] {
            let (code, stats) = optimized(byte_branch(mask, condition, false));
            assert_eq!(stats.zero_tests_narrowed, 1, "{code:?}");
            assert_eq!(extends(&code) + masks(&code), 0, "{code:?}");
            assert!(
                code.iter()
                    .any(|i| matches!(i, LowerInstr::TestLowByte { .. }))
            );
        }
    }
    // The tested value is also returned: its extension stays.
    let (code, stats) = optimized(byte_branch(false, Condition::NotEq, true));
    assert_eq!((stats.zero_tests_narrowed, extends(&code)), (0, 1));
    // Ordered conditions read more than the zero flag.
    let (_, stats) = optimized(byte_branch(true, Condition::SignedLess, false));
    assert_eq!(stats.zero_tests_narrowed, 0);
}

#[test]
fn zero_tests_see_through_single_use_copies_of_a_byte_mask() {
    let code = vec![
        argument(1, 0),
        and(2, v(1), 0xff),
        scalar(VirtualInstr::Mov {
            dst: reg(3),
            src: v(2),
        }),
        scalar(VirtualInstr::Test {
            lhs: v(3),
            rhs: imm(0),
        }),
        scalar(VirtualInstr::Jcc {
            condition: Condition::NotEq,
            target: 2,
        }),
        ret(imm(7)),
        scalar(VirtualInstr::Label(2)),
        ret(imm(9)),
    ];
    let (code, stats) = optimized(code);
    assert_eq!(stats.zero_tests_narrowed, 1);
    assert_eq!(masks(&code), 0, "{code:?}");
}
