//! Source contracts remain observable through optimization and S→L lowering.
use kfrc_body::*;
use kfrc_son::{
    ArithmeticFlags as F, FunctionGraph, GraphLevel, InterpretError, Interpreter, NodeId, Opcode,
    Outcome, SemanticOutcome, Value, ValueType, build_graph, lower_layout,
    optimize_specialized_graph, verify_graph,
};

fn graph(op: Opcode, flags: u32, signed: bool) -> FunctionGraph {
    let mut types = RecordedTypes::new();
    let ty = types.intern(
        TypeShape::Int {
            width: IntegerWidth::W8,
            signed,
        },
        Some(SizeAlign { size: 1, align: 1 }),
    );
    let mut g = FunctionGraph::new(
        "arithmetic_contract",
        FunctionAbi {
            args: vec![PassMode::Direct; 2],
            ret: PassMode::Direct,
            c_variadic: false,
        },
        types,
    );
    g.level = GraphLevel::Specialized;
    let start = g.add_node(
        Opcode::Start,
        NodeId::NONE,
        &[],
        0,
        ValueType::Tuple(vec![
            ValueType::Control,
            ValueType::Effect,
            ValueType::Rust(ty),
            ValueType::Rust(ty),
        ]),
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
    let a = g.add_node(
        Opcode::Projection,
        NodeId::NONE,
        &[start],
        2,
        ValueType::Rust(ty),
    );
    let b = g.add_node(
        Opcode::Projection,
        NodeId::NONE,
        &[start],
        3,
        ValueType::Rust(ty),
    );
    let result = g.add_node(
        op,
        if op.is_fixed() { control } else { NodeId::NONE },
        &[a, b],
        flags,
        ValueType::Rust(ty),
    );
    g.add_node(
        Opcode::Return,
        control,
        &[effect, result],
        0,
        ValueType::Control,
    );
    g
}

fn outcome(g: &FunctionGraph, a: u64, b: u64) -> Result<u64, &'static str> {
    let result = if g.level == GraphLevel::Layout {
        Interpreter::new(g)
            .run(&[a, b], &mut |name: &str, _: &[u64]| {
                Err(InterpretError::UnknownCallee(name.into()))
            })
            .map(|result| match result {
                Outcome::Return(bits) => bits,
                _ => panic!("unexpected outcome"),
            })
    } else {
        Interpreter::new(g)
            .run_values(
                &[Value::Scalar(a), Value::Scalar(b)],
                &mut |name: &str, _: &[Value]| Err(InterpretError::UnknownCallee(name.into())),
            )
            .map(|result| match result {
                SemanticOutcome::Return(Value::Scalar(bits)) => bits,
                _ => panic!("unexpected outcome"),
            })
    };
    result.map_err(|error| match error {
        InterpretError::PoisonObserved(_) => "poison",
        InterpretError::UndefinedBehaviour(_) => "trap",
        other => panic!("unexpected interpreter failure: {other:?}"),
    })
}

#[test]
fn eight_bit_contracts_match_independent_integer_arithmetic() {
    for (op, flags, signed) in [
        (Opcode::Add, F::NUW, false),
        (Opcode::Sub, F::NUW, false),
        (Opcode::Mul, F::NUW, false),
        (Opcode::Add, F::NSW, true),
        (Opcode::Sub, F::NSW, true),
        (Opcode::Mul, F::NSW, true),
        (Opcode::BitOr, F::DISJOINT, false),
        (Opcode::ShiftLeft, F::AMOUNT_IN_RANGE, false),
        (Opcode::ShiftRightArithmetic, F::AMOUNT_IN_RANGE, true),
        (Opcode::ShiftRightLogical, F::AMOUNT_IN_RANGE, false),
        (Opcode::DivUnsigned, F::EXACT, false),
        (Opcode::DivSigned, F::EXACT, true),
    ] {
        let mut original = graph(op, flags, signed);
        verify_graph(&mut original).unwrap();
        let mut optimized = original.clone();
        optimize_specialized_graph(&mut optimized).unwrap();
        let mut lowered = optimized.clone();
        lower_layout(&mut lowered).unwrap();
        for a in 0..=255u64 {
            for b in [0u64, 1, 2, 3, 7, 8, 24, 127, 128, 255] {
                let sa = i16::from(a as u8 as i8);
                let sb = i16::from(b as u8 as i8);
                let x = if signed { i32::from(sa) } else { a as i32 };
                let y = if signed { i32::from(sb) } else { b as i32 };
                let expected = match op {
                    Opcode::Add | Opcode::Sub | Opcode::Mul => {
                        let value = match op {
                            Opcode::Add => x + y,
                            Opcode::Sub => x - y,
                            _ => x * y,
                        };
                        let valid = if signed {
                            (-128..=127).contains(&value)
                        } else {
                            (0..=255).contains(&value)
                        };
                        if valid {
                            Ok(value as u64 & 255)
                        } else {
                            Err("poison")
                        }
                    }
                    Opcode::BitOr => {
                        if a & b == 0 {
                            Ok(a | b)
                        } else {
                            Err("poison")
                        }
                    }
                    Opcode::ShiftLeft
                    | Opcode::ShiftRightArithmetic
                    | Opcode::ShiftRightLogical => {
                        if b >= 8 {
                            Err("poison")
                        } else {
                            Ok(match op {
                                Opcode::ShiftLeft => a << b,
                                Opcode::ShiftRightLogical => a >> b,
                                _ => (sa >> b) as u64,
                            } & 255)
                        }
                    }
                    _ if b == 0 || (signed && sa == -128 && sb == -1) => Err("trap"),
                    _ if x % y != 0 => Err("poison"),
                    _ => Ok((x / y) as u64 & 255),
                };
                for candidate in [&original, &optimized, &lowered] {
                    assert_eq!(
                        outcome(candidate, a, b),
                        expected,
                        "{op:?}/{flags} {a}/{b} {:?}",
                        candidate.level
                    );
                }
            }
        }
    }
}

#[test]
fn flagged_constant_poison_is_not_folded_or_assumed_defined() {
    let mut g = graph(Opcode::Add, F::NUW, false);
    let add = g.ids().find(|&n| g.op(n) == Opcode::Add).unwrap();
    let ty = g.rust_type(add).unwrap();
    let max = g.add_constant(255, ty);
    let one = g.add_constant(1, ty);
    g.set_inputs(add, &[max, one]);
    let ret = g.ids().find(|&n| g.op(n) == Opcode::Return).unwrap();
    let two = g.add_constant(2, ty);
    let div = g.add_node(
        Opcode::DivUnsigned,
        g.control(ret),
        &[add, two],
        0,
        ValueType::Rust(ty),
    );
    // Fixed division must still observe the poisoned numerator even if unused.
    g.set_input(ret, 1, one);
    verify_graph(&mut g).unwrap();
    assert_eq!(outcome(&g, 0, 0), Err("poison"));
    optimize_specialized_graph(&mut g).unwrap();
    assert!(
        g.ids().any(|n| g.op(n) == Opcode::DivUnsigned),
        "removed fixed observation {div}"
    );
    assert_eq!(outcome(&g, 0, 0), Err("poison"));
}

#[test]
fn shifted_out_bits_and_shift_overflow_remain_poison_contracts() {
    for (op, flags, signed, a, b, expected) in [
        (Opcode::ShiftLeft, F::NUW, false, 128, 1, Err("poison")),
        (Opcode::ShiftLeft, F::NSW, true, 64, 1, Err("poison")),
        (Opcode::ShiftLeft, F::NSW, true, 192, 1, Ok(128)),
        (Opcode::ShiftRightLogical, F::EXACT, false, 12, 2, Ok(3)),
        (
            Opcode::ShiftRightLogical,
            F::EXACT,
            false,
            13,
            2,
            Err("poison"),
        ),
        (
            Opcode::ShiftRightArithmetic,
            F::EXACT,
            true,
            244,
            2,
            Ok(253),
        ),
        (
            Opcode::ShiftRightArithmetic,
            F::EXACT,
            true,
            245,
            2,
            Err("poison"),
        ),
    ] {
        let mut g = graph(op, flags, signed);
        verify_graph(&mut g).unwrap();
        assert_eq!(outcome(&g, a, b), expected);
        optimize_specialized_graph(&mut g).unwrap();
        assert_eq!(outcome(&g, a, b), expected);
        lower_layout(&mut g).unwrap();
        assert_eq!(outcome(&g, a, b), expected);
    }
}

#[test]
fn flags_are_opcode_specific_and_gvn_keeps_different_contracts() {
    for (op, invalid) in [
        (Opcode::Add, F::EXACT),
        (Opcode::BitOr, F::NUW),
        (Opcode::DivUnsigned, F::DISJOINT),
        (Opcode::ShiftLeft, F::EXACT),
        (Opcode::Mul, 1 << 31),
    ] {
        assert!(verify_graph(&mut graph(op, invalid, false)).is_err());
    }
    let mut g = graph(Opcode::Add, F::NUW, false);
    let add = g.ids().find(|&n| g.op(n) == Opcode::Add).unwrap();
    let wrapping = g.add_node(
        Opcode::Add,
        NodeId::NONE,
        &g.inputs(add).to_vec(),
        0,
        g.value_type(add).clone(),
    );
    let xor = g.add_node(
        Opcode::BitXor,
        NodeId::NONE,
        &[add, wrapping],
        0,
        g.value_type(add).clone(),
    );
    let ret = g.ids().find(|&n| g.op(n) == Opcode::Return).unwrap();
    g.set_input(ret, 1, xor);
    verify_graph(&mut g).unwrap();
    optimize_specialized_graph(&mut g).unwrap();
    assert_eq!(g.ids().filter(|&n| g.op(n) == Opcode::Add).count(), 2);
    assert_eq!(outcome(&g, 255, 1), Err("poison"));
}

#[test]
fn body_producer_distinguishes_unchecked_and_wrapping_operations() {
    for signed in [false, true] {
        for (op, expected) in [
            (BinaryOp::Add, 0),
            (BinaryOp::Sub, 0),
            (BinaryOp::Mul, 0),
            (BinaryOp::Shl, 0),
            (BinaryOp::Shr, 0),
            (BinaryOp::Div, 0),
            (BinaryOp::BitOr, 0),
            (BinaryOp::AddUnchecked, if signed { F::NSW } else { F::NUW }),
            (BinaryOp::SubUnchecked, if signed { F::NSW } else { F::NUW }),
            (BinaryOp::MulUnchecked, if signed { F::NSW } else { F::NUW }),
            (BinaryOp::ShlUnchecked, F::AMOUNT_IN_RANGE),
            (BinaryOp::ShrUnchecked, F::AMOUNT_IN_RANGE),
            (BinaryOp::DivExact, F::EXACT),
            (BinaryOp::BitOrDisjoint, F::DISJOINT),
        ] {
            let mut types = RecordedTypes::new();
            let ty = types.intern(
                TypeShape::Int {
                    width: IntegerWidth::W32,
                    signed,
                },
                Some(SizeAlign { size: 4, align: 4 }),
            );
            let mut b = BodyBuilder::new(
                "body_flags",
                FunctionAbi {
                    args: vec![PassMode::Direct; 2],
                    ret: PassMode::Direct,
                    c_variadic: false,
                },
                types,
            );
            for _ in 0..3 {
                b.add_local(LocalDecl {
                    ty,
                    mutability: Mutability::Mut,
                    name: None,
                });
            }
            b.set_arg_count(2);
            b.push_statement(StatementKind::Assign(
                Place::local(LocalId::RETURN),
                Rvalue::BinaryOp(
                    op,
                    Operand::Copy(Place::local(LocalId(1))),
                    Operand::Copy(Place::local(LocalId(2))),
                ),
            ));
            b.finish_block(Terminator::Return, false);
            let g = build_graph(&b.finish()).unwrap();
            let node = g.ids().find(|&n| g.op(n).is_binary_arithmetic()).unwrap();
            assert_eq!(g.payload(node), expected, "{op:?} signed={signed}");
        }
    }
}
