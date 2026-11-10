use kfrc_body::{FunctionAbi, IntegerWidth, PassMode, RecordedTypes, SizeAlign, TyId, TypeShape};
use kfrc_son::{
    ArithmeticFlags, FunctionGraph, GraphLevel, Interpreter, NodeId, Opcode, SemanticOutcome,
    Value, ValueType, optimize_graph, optimize_specialized_seams, verify_graph,
};

struct Function {
    graph: FunctionGraph,
    ty: TyId,
    args: Vec<NodeId>,
    control: NodeId,
    effect: NodeId,
}

impl Function {
    fn new(width: IntegerWidth, signed: bool, count: usize) -> Self {
        let mut types = RecordedTypes::new();
        let bytes = u64::from(width.bits(64)) / 8;
        let ty = types.intern(
            TypeShape::Int { width, signed },
            Some(SizeAlign {
                size: bytes,
                align: bytes,
            }),
        );
        let mut graph = FunctionGraph::new(
            "combine",
            FunctionAbi {
                args: vec![PassMode::Direct; count],
                ret: PassMode::Direct,
                c_variadic: false,
            },
            types,
        );
        graph.level = GraphLevel::Specialized;
        let mut outputs = vec![ValueType::Control, ValueType::Effect];
        outputs.extend(vec![ValueType::Rust(ty); count]);
        graph.start = graph.add_node(
            Opcode::Start,
            NodeId::NONE,
            &[],
            0,
            ValueType::Tuple(outputs),
        );
        let mut projection =
            |index, ty| graph.add_node(Opcode::Projection, NodeId::NONE, &[graph.start], index, ty);
        let control = projection(0, ValueType::Control);
        let effect = projection(1, ValueType::Effect);
        let args = (0..count)
            .map(|i| projection(i as u32 + 2, ValueType::Rust(ty)))
            .collect();
        Self {
            graph,
            ty,
            args,
            control,
            effect,
        }
    }

    fn constant(&mut self, n: u64) -> NodeId {
        self.graph.add_constant(u128::from(n), self.ty)
    }

    fn binary(&mut self, op: Opcode, a: NodeId, b: NodeId, flags: u32) -> NodeId {
        self.graph
            .add_node(op, NodeId::NONE, &[a, b], flags, ValueType::Rust(self.ty))
    }

    fn finish(&mut self, result: NodeId) {
        self.graph.add_node(
            Opcode::Return,
            self.control,
            &[self.effect, result],
            0,
            ValueType::Control,
        );
        verify_graph(&mut self.graph).unwrap();
    }
}

fn run(graph: &FunctionGraph, args: &[Value]) -> Result<SemanticOutcome, kfrc_son::InterpretError> {
    Interpreter::new(graph).run_values(args, &mut |_: &str, _: &[Value]| unreachable!())
}

fn count(graph: &FunctionGraph, op: Opcode) -> usize {
    graph.ids().filter(|&n| graph.op(n) == op).count()
}

#[test]
fn reversed_commutative_expressions_share_one_value() {
    for op in [
        Opcode::Add,
        Opcode::Mul,
        Opcode::BitAnd,
        Opcode::BitOr,
        Opcode::BitXor,
    ] {
        let mut f = Function::new(IntegerWidth::W64, false, 2);
        let a = f.binary(op, f.args[0], f.args[1], 0);
        let b = f.binary(op, f.args[1], f.args[0], 0);
        let result = f.binary(Opcode::Sub, a, b, 0);
        f.finish(result);
        let before = f.graph.clone();
        optimize_graph(&mut f.graph);
        verify_graph(&mut f.graph).unwrap();
        assert_eq!(count(&f.graph, op), 1, "{op:?}");
        for (a, b) in [(0, 1), (u64::MAX, 17), (1 << 63, 0x1234)] {
            let args = [Value::Scalar(a), Value::Scalar(b)];
            assert_eq!(run(&f.graph, &args), run(&before, &args));
        }
        let args = [Value::Poison, Value::Scalar(3)];
        assert!(run(&before, &args).is_err());
        assert!(run(&f.graph, &args).is_err());
    }
}

#[test]
fn nested_constants_combine_at_the_actual_integer_width() {
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
                Opcode::Mul,
                Opcode::BitAnd,
                Opcode::BitOr,
                Opcode::BitXor,
            ] {
                for reversed in [false, true] {
                    let mut f = Function::new(width, signed, 1);
                    let a = f.constant(mask - 1);
                    let b = f.constant(3);
                    let inner = if reversed {
                        f.binary(op, a, f.args[0], 0)
                    } else {
                        f.binary(op, f.args[0], a, 0)
                    };
                    let outer = if reversed {
                        f.binary(op, b, inner, 0)
                    } else {
                        f.binary(op, inner, b, 0)
                    };
                    f.finish(outer);
                    let before = f.graph.clone();
                    optimize_graph(&mut f.graph);
                    verify_graph(&mut f.graph).unwrap();
                    assert!(count(&f.graph, op) <= 1, "{width:?} {signed} {op:?}");
                    for value in [0, 1, 2, 17, mask / 2, mask - 1, mask] {
                        let args = [Value::Scalar(value)];
                        assert_eq!(run(&f.graph, &args), run(&before, &args));
                    }
                    assert!(run(&before, &[Value::Poison]).is_err());
                    assert!(run(&f.graph, &[Value::Poison]).is_err());
                    let once = f.graph.dump();
                    optimize_graph(&mut f.graph);
                    verify_graph(&mut f.graph).unwrap();
                    assert_eq!(once, f.graph.dump(), "rewrite must converge");
                }
            }
        }
    }
}

#[test]
fn reassociation_keeps_shared_inner_consumers_intact() {
    let mut f = Function::new(IntegerWidth::W8, false, 1);
    let first = f.constant(253);
    let second = f.constant(7);
    let inner = f.binary(Opcode::Add, f.args[0], first, 0);
    let outer = f.binary(Opcode::Add, inner, second, 0);
    let result = f.binary(Opcode::BitXor, inner, outer, 0);
    f.finish(result);
    let before = f.graph.clone();
    optimize_graph(&mut f.graph);
    verify_graph(&mut f.graph).unwrap();
    for input in 0..=255 {
        let args = [Value::Scalar(input)];
        assert_eq!(run(&f.graph, &args), run(&before, &args));
    }
}

#[test]
fn arithmetic_and_disjoint_contracts_block_reassociation() {
    for (op, flag) in [
        (Opcode::Add, ArithmeticFlags::NUW),
        (Opcode::Add, ArithmeticFlags::NSW),
        (Opcode::Mul, ArithmeticFlags::NUW),
        (Opcode::BitOr, ArithmeticFlags::DISJOINT),
    ] {
        for inner_flagged in [false, true] {
            let mut f = Function::new(IntegerWidth::W8, false, 1);
            let a = f.constant(127);
            let b = f.constant(3);
            let inner = f.binary(op, f.args[0], a, if inner_flagged { flag } else { 0 });
            let outer = f.binary(op, inner, b, if inner_flagged { 0 } else { flag });
            f.finish(outer);
            let before = f.graph.clone();
            optimize_graph(&mut f.graph);
            verify_graph(&mut f.graph).unwrap();
            // Keep the original contract even if a redundant, unflagged outer
            // mask can now disappear using known bits of the flagged result.
            assert!(
                f.graph
                    .ids()
                    .any(|n| f.graph.op(n) == op && f.graph.payload(n) == flag)
            );
            for input in 0..=255 {
                let args = [Value::Scalar(input)];
                match (run(&f.graph, &args), run(&before, &args)) {
                    (
                        Err(kfrc_son::InterpretError::PoisonObserved(_)),
                        Err(kfrc_son::InterpretError::PoisonObserved(_)),
                    ) => {}
                    (after, before) => assert_eq!(after, before),
                }
            }
        }
    }
}

#[test]
fn a_zero_combined_constant_does_not_hide_poison() {
    for (op, a, b) in [(Opcode::Mul, 16, 16), (Opcode::BitAnd, 15, 240)] {
        let mut f = Function::new(IntegerWidth::W8, false, 1);
        let a = f.constant(a);
        let b = f.constant(b);
        let inner = f.binary(op, f.args[0], a, 0);
        let outer = f.binary(op, inner, b, 0);
        f.finish(outer);
        optimize_graph(&mut f.graph);
        verify_graph(&mut f.graph).unwrap();
        assert_eq!(count(&f.graph, op), 1);
        assert!(run(&f.graph, &[Value::Poison]).is_err());
        assert_eq!(
            run(&f.graph, &[Value::Scalar(42)]).unwrap(),
            SemanticOutcome::Return(Value::Scalar(0))
        );
    }
}

#[test]
fn splice_seams_apply_the_same_combination_rules() {
    let mut f = Function::new(IntegerWidth::W64, false, 1);
    let a = f.constant(5);
    let b = f.constant(7);
    let inner = f.binary(Opcode::Add, f.args[0], a, 0);
    let outer = f.binary(Opcode::Add, inner, b, 0);
    f.finish(outer);
    optimize_specialized_seams(&mut f.graph, &[outer], &[]).unwrap();
    verify_graph(&mut f.graph).unwrap();
    assert_eq!(count(&f.graph, Opcode::Add), 1);
    assert_eq!(
        run(&f.graph, &[Value::Scalar(30)]).unwrap(),
        SemanticOutcome::Return(Value::Scalar(42))
    );
}

#[test]
fn known_bits_remove_shift_masks_without_discarding_poison() {
    for op in [Opcode::ShiftLeft, Opcode::ShiftRightLogical] {
        for width in [
            IntegerWidth::W8,
            IntegerWidth::W16,
            IntegerWidth::W32,
            IntegerWidth::W64,
        ] {
            let bits = width.bits(64);
            let full = u64::MAX >> (64 - bits);
            for amount in [0, 1, bits - 1, bits, bits + 1] {
                let mut f = Function::new(width, false, 1);
                let shift = f.constant(u64::from(amount));
                let value = f.binary(op, f.args[0], shift, 0);
                let k = amount % bits;
                let mask = if op == Opcode::ShiftLeft {
                    full.wrapping_shl(u32::from(k)) & full
                } else {
                    full >> k
                };
                let mask = f.constant(mask);
                let result = f.binary(Opcode::BitAnd, value, mask, 0);
                f.finish(result);
                let original = f.graph.clone();
                optimize_graph(&mut f.graph);
                verify_graph(&mut f.graph).unwrap();
                assert_eq!(count(&f.graph, Opcode::BitAnd), 0);
                for input in [0, 1, 17, full / 2, full - 1, full] {
                    assert_eq!(
                        run(&f.graph, &[Value::Scalar(input)]),
                        run(&original, &[Value::Scalar(input)])
                    );
                }
                assert!(run(&original, &[Value::Poison]).is_err());
                assert!(run(&f.graph, &[Value::Poison]).is_err());
            }
        }
    }
}

#[test]
fn known_bits_follow_bitwise_operations_and_preserve_shared_uses() {
    for op in [
        Opcode::BitAnd,
        Opcode::BitOr,
        Opcode::BitXor,
        Opcode::BitNot,
    ] {
        let mut f = Function::new(IntegerWidth::W8, false, 1);
        let low = f.constant(15);
        let limited = f.binary(Opcode::BitAnd, f.args[0], low, 0);
        let c = f.constant(3);
        let value = if op == Opcode::BitNot {
            f.graph
                .add_node(op, NodeId::NONE, &[limited], 0, ValueType::Rust(f.ty))
        } else {
            f.binary(op, limited, c, 0)
        };
        let mask = f.constant(if op == Opcode::BitNot { 240 } else { 15 });
        let masked = f.binary(
            if op == Opcode::BitNot {
                Opcode::BitOr
            } else {
                Opcode::BitAnd
            },
            value,
            mask,
            0,
        );
        let result = f.binary(Opcode::Sub, masked, value, 0);
        f.finish(result);
        let original = f.graph.clone();
        optimize_graph(&mut f.graph);
        verify_graph(&mut f.graph).unwrap();
        assert_eq!(
            count(&f.graph, Opcode::BitOr),
            usize::from(op == Opcode::BitOr)
        );
        for n in 0..=255 {
            assert_eq!(
                run(&f.graph, &[Value::Scalar(n)]),
                run(&original, &[Value::Scalar(n)])
            );
        }
        assert!(run(&f.graph, &[Value::Poison]).is_err());
    }
}

#[test]
fn known_bits_distinguish_unsigned_extension_from_signed_extension() {
    for signed in [false, true] {
        let mut f = Function::new(IntegerWidth::W8, signed, 1);
        let wide = f.graph.oracle.intern(
            TypeShape::Int {
                width: IntegerWidth::W64,
                signed: false,
            },
            Some(SizeAlign { size: 8, align: 8 }),
        );
        let cast = f.graph.add_node(
            Opcode::IntCast,
            NodeId::NONE,
            &[f.args[0]],
            0,
            ValueType::Rust(wide),
        );
        let mask = f.graph.add_constant(255, wide);
        let masked = f.graph.add_node(
            Opcode::BitAnd,
            NodeId::NONE,
            &[cast, mask],
            0,
            ValueType::Rust(wide),
        );
        f.finish(masked);
        let original = f.graph.clone();
        optimize_graph(&mut f.graph);
        verify_graph(&mut f.graph).unwrap();
        assert_eq!(count(&f.graph, Opcode::BitAnd), usize::from(signed));
        for n in 0..=255 {
            assert_eq!(
                run(&f.graph, &[Value::Scalar(n)]),
                run(&original, &[Value::Scalar(n)])
            );
        }
        assert!(run(&f.graph, &[Value::Poison]).is_err());
    }
}

#[test]
fn known_bits_keep_invalid_shift_and_disjoint_observations() {
    let mut f = Function::new(IntegerWidth::W8, false, 1);
    let shift = f.constant(1);
    let value = f.binary(Opcode::ShiftLeft, f.args[0], shift, ArithmeticFlags::NUW);
    let mask = f.constant(254);
    let masked = f.binary(Opcode::BitAnd, value, mask, 0);
    f.finish(masked);
    let original = f.graph.clone();
    optimize_graph(&mut f.graph);
    verify_graph(&mut f.graph).unwrap();
    assert_eq!(count(&f.graph, Opcode::BitAnd), 0);
    for n in 0..=255 {
        let before = run(&original, &[Value::Scalar(n)]);
        let after = run(&f.graph, &[Value::Scalar(n)]);
        assert_eq!(before.is_err(), after.is_err());
        if before.is_ok() {
            assert_eq!(before, after);
        }
    }
    let mut f = Function::new(IntegerWidth::W8, false, 1);
    let low = f.constant(15);
    let limited = f.binary(Opcode::BitAnd, f.args[0], low, 0);
    let inverse = f.graph.add_node(
        Opcode::BitNot,
        NodeId::NONE,
        &[limited],
        0,
        ValueType::Rust(f.ty),
    );
    let high = f.constant(240);
    let result = f.binary(Opcode::BitOr, inverse, high, ArithmeticFlags::DISJOINT);
    f.finish(result);
    optimize_graph(&mut f.graph);
    verify_graph(&mut f.graph).unwrap();
    assert_eq!(count(&f.graph, Opcode::BitOr), 1);
    assert!(run(&f.graph, &[Value::Scalar(0)]).is_err());
}

#[test]
fn known_sign_bits_survive_arithmetic_right_shift() {
    let mut f = Function::new(IntegerWidth::W8, true, 1);
    let sign = f.constant(128);
    let negative = f.binary(Opcode::BitOr, f.args[0], sign, 0);
    let amount = f.constant(3);
    let shifted = f.binary(Opcode::ShiftRightArithmetic, negative, amount, 0);
    let high = f.constant(224);
    let result = f.binary(Opcode::BitOr, shifted, high, 0);
    f.finish(result);
    let original = f.graph.clone();
    optimize_graph(&mut f.graph);
    verify_graph(&mut f.graph).unwrap();
    assert_eq!(count(&f.graph, Opcode::BitOr), 1);
    for n in 0..=255 {
        assert_eq!(
            run(&f.graph, &[Value::Scalar(n)]),
            run(&original, &[Value::Scalar(n)])
        );
    }
    assert!(run(&f.graph, &[Value::Poison]).is_err());
}

#[test]
fn double_not_preserves_the_entire_value_including_poison() {
    for width in [IntegerWidth::W8, IntegerWidth::W64] {
        let mut f = Function::new(width, false, 1);
        let inner = f.graph.add_node(
            Opcode::BitNot,
            NodeId::NONE,
            &[f.args[0]],
            0,
            ValueType::Rust(f.ty),
        );
        let outer = f.graph.add_node(
            Opcode::BitNot,
            NodeId::NONE,
            &[inner],
            0,
            ValueType::Rust(f.ty),
        );
        f.finish(outer);
        let before = f.graph.clone();
        optimize_graph(&mut f.graph);
        verify_graph(&mut f.graph).unwrap();
        assert!(!f.graph.ids().any(|n| f.graph.op(n) == Opcode::BitNot));
        for input in [Value::Scalar(0), Value::Scalar(129), Value::Poison] {
            let run = |g: &FunctionGraph| {
                Interpreter::new(g).run_values(&[input.clone()], &mut |_: &str, _: &[Value]| {
                    Ok(Value::Scalar(0))
                })
            };
            let a = run(&before);
            let b = run(&f.graph);
            assert_eq!(a.is_ok(), b.is_ok());
            if a.is_ok() {
                assert_eq!(a, b);
            }
        }
    }
}
