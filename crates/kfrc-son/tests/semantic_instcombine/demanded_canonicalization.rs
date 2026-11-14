use super::*;

#[test]
fn known_sign_masks_fold_before_demand_can_change_the_shift() {
    for width in [
        IntegerWidth::W8,
        IntegerWidth::W16,
        IntegerWidth::W32,
        IntegerWidth::W64,
    ] {
        let bits = width.bits(64);
        let mask = u64::MAX >> (64 - bits);
        let sign = 1u64 << (bits - 1);
        for amount in [1, 3, bits - 1] {
            for negative in [false, true] {
                let mut f = Function::new(width, true, 1);
                let op = if negative {
                    Opcode::BitOr
                } else {
                    Opcode::BitAnd
                };
                let forced = f.constant(if negative { sign } else { mask ^ sign });
                let source = f.binary(op, f.args[0], forced, 0);
                let shift = f.constant(u64::from(amount));
                let shifted = f.binary(Opcode::ShiftRightArithmetic, source, shift, 0);
                let high = mask ^ (mask >> amount);
                let retained = f.constant(if negative { high } else { mask ^ high });
                let result = f.binary(op, shifted, retained, 0);
                f.finish(result);
                let before = f.graph.clone();
                optimize_graph(&mut f.graph);
                verify_graph(&mut f.graph).unwrap();
                // KnownBits proves the outer mask redundant. Changing the
                // unobserved sign fill first must not retain a second mask.
                assert_eq!(count(&f.graph, op), 1, "{width:?} {amount} {negative}");
                assert_eq!(
                    count(&f.graph, Opcode::ShiftRightArithmetic)
                        + count(&f.graph, Opcode::ShiftRightLogical),
                    1
                );
                let values = if bits == 8 {
                    (0..=255).collect()
                } else {
                    vec![0, 1, 127, 255, sign - 1, sign, sign + 1, mask]
                };
                casts_and_demand::check_values(&before, &f.graph, &values);
                assert_converges(&mut f.graph);
            }
        }
    }
}

fn scalar_type(f: &mut Function, shape: TypeShape) -> TyId {
    let size = match shape {
        TypeShape::Bool => 1,
        TypeShape::Char => 4,
        TypeShape::Int { width, .. } => u64::from(width.bits(64)) / 8,
        _ => unreachable!(),
    };
    f.graph
        .oracle
        .intern(shape, Some(SizeAlign { size, align: size }))
}

#[test]
fn bool_and_char_cast_chains_keep_their_bits_and_result_type() {
    use IntegerWidth::*;
    let integer = |width, signed| TypeShape::Int { width, signed };
    for (source, middle, target, casts, masks) in [
        (TypeShape::Bool, integer(W8, true), integer(W64, true), 1, 0),
        (TypeShape::Bool, integer(W64, false), TypeShape::Bool, 0, 0),
        (
            TypeShape::Char,
            integer(W32, false),
            integer(W64, false),
            1,
            0,
        ),
        (TypeShape::Char, integer(W64, true), TypeShape::Char, 0, 0),
        (TypeShape::Char, integer(W8, false), TypeShape::Char, 0, 1),
        (
            integer(W8, false),
            TypeShape::Char,
            integer(W64, false),
            1,
            0,
        ),
        (
            integer(W8, false),
            TypeShape::Bool,
            integer(W64, false),
            2,
            0,
        ),
        (
            integer(W8, false),
            integer(W32, false),
            TypeShape::Char,
            1,
            0,
        ),
    ] {
        let values = match source {
            TypeShape::Bool => vec![0, 1],
            TypeShape::Char => vec![0, 1, 0x7f, 0xff, 0xd7ff, 0xe000, 0x10000, 0x1d800, 0x10ffff],
            _ => (0..=255).collect(),
        };
        let mut f = Function::new(W8, false, 1);
        let source = scalar_type(&mut f, source);
        f.ty = source;
        f.graph.set_value_type(f.args[0], ValueType::Rust(source));
        f.graph.set_value_type(
            f.graph.start,
            ValueType::Tuple(vec![
                ValueType::Control,
                ValueType::Effect,
                ValueType::Rust(source),
            ]),
        );
        let middle = scalar_type(&mut f, middle);
        let target = scalar_type(&mut f, target);
        let argument = f.args[0];
        let intermediate = casts_and_demand::cast(&mut f, argument, middle);
        let result = casts_and_demand::cast(&mut f, intermediate, target);
        f.finish(result);
        let before = f.graph.clone();
        optimize_graph(&mut f.graph);
        verify_graph(&mut f.graph).unwrap();
        assert_eq!(count(&f.graph, Opcode::IntCast), casts);
        assert_eq!(count(&f.graph, Opcode::BitAnd), masks);
        let ret = f
            .graph
            .ids()
            .find(|&n| f.graph.op(n) == Opcode::Return)
            .unwrap();
        assert_eq!(f.graph.rust_type(f.graph.input(ret, 1)), Some(target));
        casts_and_demand::check_values(&before, &f.graph, &values);
        assert_converges(&mut f.graph);
    }
}
