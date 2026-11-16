use super::*;

fn ty(f: &mut Function, width: IntegerWidth, signed: bool) -> TyId {
    let bytes = u64::from(width.bits(64)) / 8;
    f.graph.oracle.intern(
        TypeShape::Int { width, signed },
        Some(SizeAlign {
            size: bytes,
            align: bytes,
        }),
    )
}

pub(super) fn cast(f: &mut Function, source: NodeId, target: TyId) -> NodeId {
    f.graph.add_node(
        Opcode::IntCast,
        NodeId::NONE,
        &[source],
        0,
        ValueType::Rust(target),
    )
}

fn pair(f: &mut Function, a: NodeId, b: NodeId) -> NodeId {
    let fields = vec![f.graph.rust_type(a).unwrap(), f.graph.rust_type(b).unwrap()];
    let ty = f.graph.oracle.intern(TypeShape::Tuple(fields), None);
    f.graph.add_node(
        Opcode::Aggregate,
        NodeId::NONE,
        &[a, b],
        0,
        ValueType::Rust(ty),
    )
}

pub(super) fn check_values(before: &FunctionGraph, after: &FunctionGraph, values: &[u64]) {
    for &value in values {
        let args = [Value::Scalar(value)];
        let original = run(before, &args);
        let optimized = run(after, &args);
        assert_eq!(original.is_err(), optimized.is_err(), "input={value:#x}");
        if original.is_ok() {
            assert_eq!(original, optimized, "input={value:#x}");
        }
    }
    assert!(run(before, &[Value::Poison]).is_err());
    assert!(run(after, &[Value::Poison]).is_err());
}

#[test]
fn cast_chains_distinguish_signed_extension_and_lost_bits() {
    use IntegerWidth::*;
    for (source, middle, target, remaining) in [
        ((W8, false), (W16, true), (W64, false), 1),
        ((W8, true), (W16, false), (W64, false), 2),
        ((W8, true), (W8, false), (W64, true), 2),
        ((W16, false), (W8, false), (W64, false), 2),
        ((W64, false), (W32, true), (W16, true), 1),
        ((W16, true), (W64, false), (W32, true), 1),
        ((W32, true), (W64, true), (W32, true), 0),
    ] {
        let mut f = Function::new(source.0, source.1, 1);
        let middle = ty(&mut f, middle.0, middle.1);
        let target = ty(&mut f, target.0, target.1);
        let input = f.args[0];
        let intermediate = cast(&mut f, input, middle);
        let result = cast(&mut f, intermediate, target);
        f.finish(result);
        let before = f.graph.clone();
        optimize_graph(&mut f.graph);
        verify_graph(&mut f.graph).unwrap();
        assert_eq!(count(&f.graph, Opcode::IntCast), remaining);
        check_values(
            &before,
            &f.graph,
            &[0, 1, 127, 128, 255, 256, 32768, u64::MAX],
        );
        assert_converges(&mut f.graph);
    }
}

#[test]
fn cast_chains_differential_at_every_integer_width_and_signedness() {
    let widths = [
        IntegerWidth::W8,
        IntegerWidth::W16,
        IntegerWidth::W32,
        IntegerWidth::W64,
    ];
    for source in widths {
        for middle in widths {
            for target in widths {
                for signs in 0..8 {
                    let mut f = Function::new(source, signs & 1 != 0, 1);
                    let middle = ty(&mut f, middle, signs & 2 != 0);
                    let target = ty(&mut f, target, signs & 4 != 0);
                    let input = f.args[0];
                    let intermediate = cast(&mut f, input, middle);
                    let result = cast(&mut f, intermediate, target);
                    f.finish(result);
                    let before = f.graph.clone();
                    optimize_graph(&mut f.graph);
                    verify_graph(&mut f.graph).unwrap();
                    check_values(
                        &before,
                        &f.graph,
                        &[
                            0,
                            1,
                            127,
                            128,
                            255,
                            256,
                            32767,
                            32768,
                            65535,
                            1 << 31,
                            1 << 63,
                            0x0123_4567_89ab_cdef,
                            u64::MAX,
                        ],
                    );
                }
            }
        }
    }
}

#[test]
fn cast_chain_keeps_the_source_overflow_promise_and_shared_inner_use() {
    let mut f = Function::new(IntegerWidth::W8, false, 1);
    let limit = f.constant(128);
    let source = f.binary(Opcode::Add, f.args[0], limit, ArithmeticFlags::NUW);
    let wide = ty(&mut f, IntegerWidth::W32, false);
    let intermediate = cast(&mut f, source, wide);
    let narrow = f.ty;
    let result = cast(&mut f, intermediate, narrow);
    let both = pair(&mut f, result, intermediate);
    f.finish(both);
    let before = f.graph.clone();
    optimize_graph(&mut f.graph);
    verify_graph(&mut f.graph).unwrap();
    assert_eq!(count(&f.graph, Opcode::IntCast), 1);
    assert!(
        f.graph.ids().any(|n| {
            f.graph.op(n) == Opcode::Add && f.graph.payload(n) == ArithmeticFlags::NUW
        })
    );
    check_values(&before, &f.graph, &(0..=255).collect::<Vec<_>>());
}

#[test]
fn demanded_masks_propagate_through_truncation_and_constant_shifts() {
    for (op, mask) in [
        (Opcode::BitAnd, 0xffff),
        (Opcode::BitOr, 0xffff_0000),
        (Opcode::BitXor, 0xffff_0000),
    ] {
        let mut f = Function::new(IntegerWidth::W32, false, 1);
        let bits = f.constant(mask);
        let masked = f.binary(op, f.args[0], bits, 0);
        let amount = f.constant(8);
        let shifted = f.binary(Opcode::ShiftRightLogical, masked, amount, 0);
        let byte = ty(&mut f, IntegerWidth::W8, false);
        let result = cast(&mut f, shifted, byte);
        f.finish(result);
        let before = f.graph.clone();
        optimize_graph(&mut f.graph);
        verify_graph(&mut f.graph).unwrap();
        assert_eq!(count(&f.graph, op), 0);
        check_values(
            &before,
            &f.graph,
            &[0, 1, 255, 256, 32768, 65535, 0xabcd_1234, u64::MAX],
        );
    }
}

#[test]
fn multiple_narrow_uses_merge_their_bit_masks() {
    let mut f = Function::new(IntegerWidth::W32, false, 1);
    let bits = f.constant(0xffff);
    let shared = f.binary(Opcode::BitAnd, f.args[0], bits, 0);
    let byte = ty(&mut f, IntegerWidth::W8, false);
    let low = cast(&mut f, shared, byte);
    let amount = f.constant(8);
    let shifted = f.binary(Opcode::ShiftRightLogical, shared, amount, 0);
    let high = cast(&mut f, shifted, byte);
    let result = pair(&mut f, low, high);
    f.finish(result);
    let before = f.graph.clone();
    optimize_graph(&mut f.graph);
    verify_graph(&mut f.graph).unwrap();
    assert_eq!(count(&f.graph, Opcode::BitAnd), 0);
    check_values(
        &before,
        &f.graph,
        &[0, 255, 256, 32768, 65535, 0xabcd_1234, u64::MAX],
    );
}

#[test]
fn full_width_use_protects_the_shared_producer() {
    let mut f = Function::new(IntegerWidth::W32, false, 1);
    let bits = f.constant(0xffff);
    let shared = f.binary(Opcode::BitAnd, f.args[0], bits, 0);
    let byte = ty(&mut f, IntegerWidth::W8, false);
    let low = cast(&mut f, shared, byte);
    let boolean = f
        .graph
        .oracle
        .intern(TypeShape::Bool, Some(SizeAlign { size: 1, align: 1 }));
    let expected = f.constant(0x1234);
    let comparison = f.graph.add_node(
        Opcode::Compare,
        NodeId::NONE,
        &[shared, expected],
        kfrc_son::ComparePredicate::Equal as u32,
        ValueType::Rust(boolean),
    );
    let result = pair(&mut f, low, comparison);
    f.finish(result);
    let before = f.graph.clone();
    optimize_graph(&mut f.graph);
    verify_graph(&mut f.graph).unwrap();
    assert_eq!(count(&f.graph, Opcode::BitAnd), 1);
    // Only the cast's input bypasses the mask; the full-width comparison
    // still depends on the original masked producer.
    let cast = f
        .graph
        .ids()
        .find(|&n| f.graph.op(n) == Opcode::IntCast)
        .unwrap();
    assert_eq!(f.graph.op(f.graph.input(cast, 0)), Opcode::Projection);
    check_values(
        &before,
        &f.graph,
        &[
            0,
            255,
            0x1234,
            65535,
            65536,
            0x1_1234,
            0xabcd_1234,
            u64::MAX,
        ],
    );
}

#[test]
fn signed_shift_demand_includes_the_sign_bit_and_respects_modulo_amounts() {
    for (amount, narrow) in [(8, true), (40, true), (31, true), (8, false)] {
        let mut f = Function::new(IntegerWidth::W32, true, 1);
        let shift = f.constant(amount);
        let shifted = f.binary(Opcode::ShiftRightArithmetic, f.args[0], shift, 0);
        let target = ty(
            &mut f,
            if narrow {
                IntegerWidth::W8
            } else {
                IntegerWidth::W32
            },
            true,
        );
        let result = cast(&mut f, shifted, target);
        f.finish(result);
        let before = f.graph.clone();
        optimize_graph(&mut f.graph);
        verify_graph(&mut f.graph).unwrap();
        // For >>31 the returned byte contains sign-filled bits. For a full
        // word result every sign-filled bit remains observable.
        let logical = narrow && amount % 32 <= 24;
        assert_eq!(
            count(&f.graph, Opcode::ShiftRightArithmetic),
            usize::from(!logical)
        );
        check_values(
            &before,
            &f.graph,
            &[0, 1, 127, 255, 0x7fff_ffff, 0x8000_0000, u64::MAX],
        );
    }
}

#[test]
fn unsigned_truncation_round_trip_becomes_a_mask() {
    for signed in [false, true] {
        let mut f = Function::new(IntegerWidth::W64, signed, 1);
        let byte = ty(&mut f, IntegerWidth::W8, false);
        let input = f.args[0];
        let narrow = cast(&mut f, input, byte);
        let original = f.ty;
        let result = cast(&mut f, narrow, original);
        f.finish(result);
        let before = f.graph.clone();
        optimize_graph(&mut f.graph);
        verify_graph(&mut f.graph).unwrap();
        assert_eq!(count(&f.graph, Opcode::IntCast), 0);
        assert_eq!(count(&f.graph, Opcode::BitAnd), 1);
        check_values(
            &before,
            &f.graph,
            &[0, 127, 128, 255, 256, 1 << 63, u64::MAX],
        );
    }
}

#[test]
fn demanded_rewrites_keep_shift_and_disjoint_poison_contracts() {
    for (op, flag, literal) in [
        (Opcode::ShiftRightArithmetic, ArithmeticFlags::EXACT, 8),
        (
            Opcode::ShiftRightArithmetic,
            ArithmeticFlags::AMOUNT_IN_RANGE,
            40,
        ),
        (Opcode::ShiftLeft, ArithmeticFlags::NUW, 8),
        (Opcode::ShiftLeft, ArithmeticFlags::NSW, 8),
        (Opcode::BitOr, ArithmeticFlags::DISJOINT, 0xffff_0000),
    ] {
        let mut f = Function::new(IntegerWidth::W32, true, 1);
        let bits = f.constant(literal);
        let promised = f.binary(op, f.args[0], bits, flag);
        let byte = ty(&mut f, IntegerWidth::W8, false);
        let result = cast(&mut f, promised, byte);
        f.finish(result);
        let before = f.graph.clone();
        optimize_graph(&mut f.graph);
        verify_graph(&mut f.graph).unwrap();
        assert!(
            f.graph
                .ids()
                .any(|n| f.graph.op(n) == op && f.graph.payload(n) == flag)
        );
        check_values(
            &before,
            &f.graph,
            &[0, 1, 255, 256, 32768, 65535, 0x8000_0000, u64::MAX],
        );
    }
}

#[test]
fn shift_uses_discard_only_the_unobserved_input_bits() {
    for (op, mask) in [
        (Opcode::ShiftLeft, 0x00ff_ffff),
        (Opcode::ShiftRightLogical, 0xffff_ff00),
    ] {
        let mut f = Function::new(IntegerWidth::W32, false, 1);
        let mask = f.constant(mask);
        let masked = f.binary(Opcode::BitAnd, f.args[0], mask, 0);
        let amount = f.constant(8);
        let result = f.binary(op, masked, amount, 0);
        f.finish(result);
        let before = f.graph.clone();
        optimize_graph(&mut f.graph);
        verify_graph(&mut f.graph).unwrap();
        assert_eq!(count(&f.graph, Opcode::BitAnd), 0);
        check_values(&before, &f.graph, &[0, 1, 255, 256, 0xabcd_1234, u64::MAX]);
    }
}

#[test]
fn poison_promises_observe_bits_outside_the_narrow_result() {
    for (op, flag, mask, literal) in [
        (Opcode::ShiftLeft, ArithmeticFlags::NUW, 0x00ff_ffff, 8),
        (
            Opcode::ShiftRightArithmetic,
            ArithmeticFlags::EXACT,
            0xffff_ff00,
            8,
        ),
        (
            Opcode::BitOr,
            ArithmeticFlags::DISJOINT,
            0xffff,
            0xffff_0000,
        ),
    ] {
        let mut f = Function::new(IntegerWidth::W32, false, 1);
        let mask = f.constant(mask);
        let masked = f.binary(Opcode::BitAnd, f.args[0], mask, 0);
        let literal = f.constant(literal);
        let promised = f.binary(op, masked, literal, flag);
        let byte = ty(&mut f, IntegerWidth::W8, false);
        let result = cast(&mut f, promised, byte);
        f.finish(result);
        let before = f.graph.clone();
        optimize_graph(&mut f.graph);
        verify_graph(&mut f.graph).unwrap();
        assert_eq!(count(&f.graph, Opcode::BitAnd), 1);
        check_values(&before, &f.graph, &[0, 1, 255, 256, 0xabcd_1234, u64::MAX]);
    }
}

#[test]
fn seeded_rewrite_handles_casts_and_only_the_selected_use() {
    let mut f = Function::new(IntegerWidth::W32, false, 1);
    let mask = f.constant(0xffff);
    let shared = f.binary(Opcode::BitAnd, f.args[0], mask, 0);
    let byte = ty(&mut f, IntegerWidth::W8, false);
    let low = cast(&mut f, shared, byte);
    let result = pair(&mut f, low, shared);
    f.finish(result);
    let before = f.graph.clone();
    optimize_specialized_seams(&mut f.graph, &[low], &[]).unwrap();
    verify_graph(&mut f.graph).unwrap();
    assert_eq!(count(&f.graph, Opcode::BitAnd), 1);
    check_values(&before, &f.graph, &[0, 255, 65535, 65536, u64::MAX]);
}
