use super::*;
use crate::ArithmeticFlags;

fn integer_fixture(width: IntegerWidth, signed: bool) -> Fixture {
    let mut f = Fixture::new(false);
    let size = u64::from(width.bits(64)) / 8;
    f.word = f.graph.oracle.intern(
        TypeShape::Int { width, signed },
        Some(SizeAlign { size, align: size }),
    );
    f.graph.set_value_type(f.arg, ValueType::Rust(f.word));
    f.graph.set_value_type(
        f.graph.start,
        ValueType::Tuple(vec![
            ValueType::Control,
            ValueType::Effect,
            ValueType::Rust(f.word),
        ]),
    );
    f
}

#[test]
fn defined_scalar_operations_remove_only_redundant_observations() {
    for width in [
        IntegerWidth::W8,
        IntegerWidth::W16,
        IntegerWidth::W32,
        IntegerWidth::W64,
    ] {
        let mask = u64::MAX >> (64 - width.bits(64));
        for signed in [false, true] {
            for op in [
                Opcode::Add,
                Opcode::Sub,
                Opcode::Mul,
                Opcode::MulOverflow,
                Opcode::BitAnd,
                Opcode::BitOr,
                Opcode::BitXor,
                Opcode::ShiftLeft,
                Opcode::ShiftRightLogical,
                Opcode::ShiftRightArithmetic,
                Opcode::Negate,
                Opcode::BitNot,
                Opcode::IntCast,
                Opcode::Compare,
            ] {
                let mut f = integer_fixture(width, signed);
                let amount = f.graph.add_constant(u128::from(width.bits(64) + 1), f.word);
                let unary = matches!(op, Opcode::Negate | Opcode::BitNot | Opcode::IntCast);
                let ty = if matches!(op, Opcode::Compare | Opcode::MulOverflow) {
                    f.boolean
                } else {
                    f.word
                };
                let operands = if unary {
                    vec![f.arg]
                } else {
                    vec![f.arg, amount]
                };
                let value = f
                    .graph
                    .add_node(op, NodeId::NONE, &operands, 0, ValueType::Rust(ty));
                f.observe(value);
                f.call(false, false);
                f.ret(f.arg);
                let before = f.graph.clone();
                assert_eq!(
                    f.optimize().observations_removed,
                    1,
                    "{width:?} {signed} {op:?}"
                );
                assert_eq!(f.count(), 0);
                for argument in [0, 1, mask / 2, mask - 1, mask] {
                    assert_eq!(
                        execute(&before, Value::Scalar(argument), false),
                        execute(&f.graph, Value::Scalar(argument), false),
                        "{width:?} {signed} {op:?} {argument:#x}"
                    );
                }
                assert_eq!(
                    execute(&before, Value::Poison, false),
                    execute(&f.graph, Value::Poison, false)
                );
            }
        }
    }
}

#[test]
fn scalar_definedness_composes_across_casts_comparisons_and_bool_not() {
    let mut f = integer_fixture(IntegerWidth::W8, true);
    let wide = f.graph.oracle.intern(
        TypeShape::Int {
            width: IntegerWidth::W64,
            signed: true,
        },
        Some(SizeAlign { size: 8, align: 8 }),
    );
    let cast = f.graph.add_node(
        Opcode::IntCast,
        NodeId::NONE,
        &[f.arg],
        0,
        ValueType::Rust(wide),
    );
    let zero = f.graph.add_constant(0, wide);
    let condition = f.graph.add_node(
        Opcode::Compare,
        NodeId::NONE,
        &[cast, zero],
        ComparePredicate::SignedGreaterEqual as u32,
        ValueType::Rust(f.boolean),
    );
    let inverted = f.graph.add_node(
        Opcode::BitNot,
        NodeId::NONE,
        &[condition],
        0,
        ValueType::Rust(f.boolean),
    );
    f.observe(inverted);
    f.ret(f.arg);
    let before = f.graph.clone();
    assert_eq!(f.optimize().observations_removed, 1);
    for argument in 0..=255 {
        assert_eq!(
            execute(&before, Value::Scalar(argument), false),
            execute(&f.graph, Value::Scalar(argument), false)
        );
    }
}

#[test]
fn poison_producing_contracts_and_undefined_inputs_keep_their_observation() {
    for (op, payload) in [
        (Opcode::Add, ArithmeticFlags::NUW),
        (Opcode::Add, ArithmeticFlags::NSW),
        (Opcode::Sub, ArithmeticFlags::NUW),
        (Opcode::Mul, ArithmeticFlags::NSW),
        (Opcode::BitOr, ArithmeticFlags::DISJOINT),
        (Opcode::ShiftLeft, ArithmeticFlags::AMOUNT_IN_RANGE),
        (Opcode::ShiftRightLogical, ArithmeticFlags::EXACT),
    ] {
        let mut f = integer_fixture(IntegerWidth::W8, false);
        let rhs = f.graph.add_constant(9, f.word);
        let value = f.graph.add_node(
            op,
            NodeId::NONE,
            &[f.arg, rhs],
            payload,
            ValueType::Rust(f.word),
        );
        f.observe(value);
        f.call(false, false);
        f.ret(f.arg);
        let before = f.graph.clone();
        assert_eq!(f.optimize().observations_removed, 0);
        for argument in 0..=255 {
            assert_eq!(
                execute(&before, Value::Scalar(argument), false),
                execute(&f.graph, Value::Scalar(argument), false)
            );
        }
    }
    let mut f = integer_fixture(IntegerWidth::W8, false);
    let poison = f.poison(f.word);
    let value = f.graph.add_node(
        Opcode::Add,
        NodeId::NONE,
        &[f.arg, poison],
        0,
        ValueType::Rust(f.word),
    );
    f.observe(value);
    f.call(false, false);
    f.ret(f.arg);
    let before = f.graph.clone();
    assert_eq!(f.optimize().observations_removed, 0);
    let result = execute(&f.graph, Value::Scalar(1), false);
    assert!(matches!(result.0, Err(InterpretError::PoisonObserved(_))));
    assert!(result.1.is_empty());
    assert_eq!(result, execute(&before, Value::Scalar(1), false));
}

#[test]
fn representation_decoding_errors_are_not_defined_scalar_inputs() {
    let mut f = Fixture::new(false);
    let bad = f.graph.add_constant(2, f.boolean);
    f.graph.set_opcode(bad, Opcode::ConstRepr);
    let value = f.graph.add_node(
        Opcode::BitNot,
        NodeId::NONE,
        &[bad],
        0,
        ValueType::Rust(f.boolean),
    );
    f.observe(value);
    f.call(false, false);
    f.ret(f.arg);
    let before = f.graph.clone();
    assert_eq!(f.optimize().observations_removed, 0);
    let result = execute(&f.graph, Value::Scalar(1), false);
    assert!(matches!(
        result.0,
        Err(InterpretError::UndefinedBehaviour(_))
    ));
    assert!(result.1.is_empty());
    assert_eq!(result, execute(&before, Value::Scalar(1), false));
}
