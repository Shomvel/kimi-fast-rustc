use kfrc_body::{FunctionAbi, IntegerWidth, PassMode, RecordedTypes, SizeAlign, TypeShape};
use kfrc_son::{FunctionGraph, NodeId, Opcode, ValueType, lower_layout, verify_graph};
use kfrc_x86::machine::{
    eval,
    x86::{BitWidth, Reg},
};
use kfrc_x86::{MachineFunction, MachineInstr, lower_function};

fn graph(op: Opcode, payload: u32, bits: u32, signed: bool, rotate: bool) -> MachineFunction {
    let mut types = RecordedTypes::new();
    let width = match bits {
        8 => IntegerWidth::W8,
        16 => IntegerWidth::W16,
        32 => IntegerWidth::W32,
        64 => IntegerWidth::W64,
        _ => unreachable!(),
    };
    let ty = types.intern(
        TypeShape::Int { width, signed },
        Some(SizeAlign {
            size: u64::from(bits / 8),
            align: u64::from(bits / 8),
        }),
    );
    let word = types.intern(
        TypeShape::Int {
            width: IntegerWidth::W64,
            signed: false,
        },
        Some(SizeAlign { size: 8, align: 8 }),
    );
    let count = types.intern(
        TypeShape::Int {
            width: IntegerWidth::W32,
            signed: false,
        },
        Some(SizeAlign { size: 4, align: 4 }),
    );
    let nargs = if op == Opcode::FunnelShift {
        3
    } else if matches!(op, Opcode::SaturatingAdd | Opcode::SaturatingSub) {
        2
    } else {
        1
    };
    let mut g = FunctionGraph::new(
        "intrinsic",
        FunctionAbi {
            args: vec![PassMode::Direct; nargs],
            ret: PassMode::Direct,
            c_variadic: false,
        },
        types,
    );
    let mut outputs = vec![ValueType::Control, ValueType::Effect];
    for i in 0..nargs {
        outputs.push(ValueType::Rust(if i == 2 { word } else { ty }));
    }
    let start = g.add_node(
        Opcode::Start,
        NodeId::NONE,
        &[],
        0,
        ValueType::Tuple(outputs),
    );
    g.start = start;
    let control = g.add_node(
        Opcode::Projection,
        NodeId::NONE,
        &[start],
        0,
        ValueType::Control,
    );
    let effect = g.add_node(
        Opcode::Projection,
        NodeId::NONE,
        &[start],
        1,
        ValueType::Effect,
    );
    let mut args: Vec<_> = (0..nargs)
        .map(|i| {
            g.add_node(
                Opcode::Projection,
                NodeId::NONE,
                &[start],
                i as u32 + 2,
                ValueType::Rust(if i == 2 { word } else { ty }),
            )
        })
        .collect();
    if rotate {
        args[1] = args[0];
    }
    let result = g.add_node(
        op,
        NodeId::NONE,
        &args,
        payload,
        ValueType::Rust(if op == Opcode::BitCount { count } else { ty }),
    );
    g.add_node(
        Opcode::Return,
        control,
        &[effect, result],
        0,
        ValueType::Control,
    );
    verify_graph(&mut g).unwrap();
    lower_layout(&mut g).unwrap();
    verify_graph(&mut g).unwrap();
    let machine = lower_function(&g).unwrap();
    machine.emit_machine_code().unwrap();
    let asm = machine.emit_gnu_assembly().unwrap();
    for forbidden in ["popcnt", "lzcnt", "tzcnt"] {
        assert!(!asm.contains(forbidden), "{asm}");
    }
    machine
}
fn canonical(value: u64, bits: u32, signed: bool) -> i64 {
    if signed {
        ((value << (64 - bits)) as i64) >> (64 - bits)
    } else {
        (value & (u64::MAX >> (64 - bits))) as i64
    }
}

#[test]
fn counts_and_bit_permutations_cover_all_widths_and_zero() {
    for bits in [8, 16, 32, 64] {
        for signed in [false, true] {
            for (op, payload) in [
                (Opcode::BitCount, 0),
                (Opcode::BitCount, 1),
                (Opcode::BitCount, 2),
                (Opcode::ByteSwap, 0),
                (Opcode::BitReverse, 0),
            ] {
                let f = graph(op, payload, bits, signed, false);
                let mask = u64::MAX >> (64 - bits);
                for x in (0..=255).chain([
                    1u64 << (bits - 1),
                    mask,
                    mask - 1,
                    0x0123456789abcdef & mask,
                ]) {
                    let x = x & mask;
                    let expected = match op {
                        Opcode::BitCount => i64::from(match payload {
                            0 => x.count_ones(),
                            1 => x.leading_zeros() - (64 - bits),
                            _ => x.trailing_zeros().min(bits),
                        }),
                        Opcode::ByteSwap => canonical(x.swap_bytes() >> (64 - bits), bits, signed),
                        Opcode::BitReverse => {
                            canonical(x.reverse_bits() >> (64 - bits), bits, signed)
                        }
                        _ => unreachable!(),
                    };
                    assert_eq!(
                        eval::evaluate(&f, &[canonical(x, bits, signed)]),
                        Ok(expected),
                        "{op:?}/{payload} bits={bits} x={x}"
                    );
                }
            }
        }
    }
}

#[test]
fn funnel_and_native_rotate_use_original_amount_modulo_data_width() {
    for bits in [8, 16, 32, 64] {
        for right in [false, true] {
            for rotate in [false, true] {
                let f = graph(Opcode::FunnelShift, u32::from(right), bits, false, rotate);
                if rotate {
                    assert!(
                        f.instructions
                            .iter()
                            .any(|i| matches!(i, MachineInstr::Rotate { .. }))
                    );
                }
                let mask = u64::MAX >> (64 - bits);
                for (a, b) in [
                    (0, 0),
                    (1, 2),
                    (mask, 0),
                    (0xabcdef1234567890 & mask, 0x1234567890abcdef & mask),
                ] {
                    for amount in [
                        0,
                        1,
                        bits as u64 - 1,
                        bits as u64,
                        bits as u64 + 1,
                        255,
                        256,
                        u64::MAX,
                    ] {
                        let b = if rotate { a } else { b };
                        let n = (amount % u64::from(bits)) as u32;
                        let result = if n == 0 {
                            if right { b } else { a }
                        } else if right {
                            (b >> n) | (a << (bits - n))
                        } else {
                            (a << n) | (b >> (bits - n))
                        };
                        assert_eq!(
                            eval::evaluate(&f, &[a as i64, b as i64, amount as i64]),
                            Ok((result & mask) as i64),
                            "bits={bits} right={right} rotate={rotate} amount={amount}"
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn saturation_preserves_signed_and_unsigned_boundaries() {
    for bits in [8, 16, 32, 64] {
        for signed in [false, true] {
            for sub in [false, true] {
                let f = graph(
                    if sub {
                        Opcode::SaturatingSub
                    } else {
                        Opcode::SaturatingAdd
                    },
                    0,
                    bits,
                    signed,
                    false,
                );
                let mask = u64::MAX >> (64 - bits);
                let values = [0, 1, 2, mask >> 1, (mask >> 1) + 1, mask - 1, mask];
                for a in values {
                    for b in values {
                        let a = canonical(a, bits, signed);
                        let b = canonical(b, bits, signed);
                        let (lo, hi) = if signed {
                            (-(1i128 << (bits - 1)), (1i128 << (bits - 1)) - 1)
                        } else {
                            (0, (1i128 << bits) - 1)
                        };
                        let a128 = if signed {
                            i128::from(a)
                        } else {
                            i128::from(a as u64)
                        };
                        let b128 = if signed {
                            i128::from(b)
                        } else {
                            i128::from(b as u64)
                        };
                        let expected =
                            (if sub { a128 - b128 } else { a128 + b128 }).clamp(lo, hi) as i64;
                        assert_eq!(
                            eval::evaluate(&f, &[a, b]),
                            Ok(expected),
                            "bits={bits} signed={signed} sub={sub} a={a} b={b}"
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn native_bit_instruction_encodings_use_baseline_opcodes_and_rex_widths() {
    let mut f = MachineFunction::new("native_bits");
    f.instructions = vec![
        MachineInstr::Rotate {
            dst: Reg::R8,
            width: BitWidth::W8,
            right: false,
            count: None,
        },
        MachineInstr::Rotate {
            dst: Reg::R9,
            width: BitWidth::W16,
            right: true,
            count: Some(8),
        },
        MachineInstr::Rotate {
            dst: Reg::R10,
            width: BitWidth::W32,
            right: false,
            count: None,
        },
        MachineInstr::Rotate {
            dst: Reg::R11,
            width: BitWidth::W64,
            right: true,
            count: None,
        },
        MachineInstr::ByteSwap {
            dst: Reg::R8,
            wide: false,
        },
        MachineInstr::ByteSwap {
            dst: Reg::R9,
            wide: true,
        },
        MachineInstr::BitScanZero {
            dst: Reg::R8,
            src: Reg::R9,
            zero: Reg::R10,
            reverse: false,
        },
        MachineInstr::Ret,
    ];
    assert_eq!(
        f.emit_machine_code().unwrap(),
        [
            0x41, 0xd2, 0xc0, 0x66, 0x41, 0xc1, 0xc9, 8, 0x41, 0xd3, 0xc2, 0x49, 0xd3, 0xcb, 0x41,
            0x0f, 0xc8, 0x49, 0x0f, 0xc9, 0x4d, 0x0f, 0xbc, 0xc1, 0x4d, 0x0f, 0x44, 0xc2, 0xc3
        ]
    );
}

#[test]
fn zero_poison_counts_scan_nonzero_inputs_without_a_constant_fallback() {
    use kfrc_son::IntegerIntrinsicPayload as P;
    for bits in [8, 16, 32, 64] {
        for signed in [false, true] {
            for mode in [P::LEADING_ZEROS, P::TRAILING_ZEROS] {
                let f = graph(Opcode::BitCount, mode | P::ZERO_POISON, bits, signed, false);
                let constants = f
                    .instructions
                    .iter()
                    .filter(|i| matches!(i, kfrc_x86::MachineInstr::MovImm { .. }))
                    .count();
                let total = graph(Opcode::BitCount, mode, bits, signed, false)
                    .instructions
                    .iter()
                    .filter(|i| matches!(i, kfrc_x86::MachineInstr::MovImm { .. }))
                    .count();
                assert!(constants < total, "mode {mode} bits {bits}: {f:?}");
                let mask = u64::MAX >> (64 - bits);
                // A zero input is poison; only nonzero inputs are defined.
                for x in (1..=255).chain([1u64 << (bits - 1), mask, 0x0123456789abcdef & mask]) {
                    let x = x & mask;
                    if x == 0 {
                        continue;
                    }
                    let expected = i64::from(if mode == P::LEADING_ZEROS {
                        x.leading_zeros() - (64 - bits)
                    } else {
                        x.trailing_zeros()
                    });
                    assert_eq!(
                        eval::evaluate(&f, &[canonical(x, bits, signed)]),
                        Ok(expected),
                        "mode {mode} bits={bits} x={x}"
                    );
                }
            }
        }
    }
}
