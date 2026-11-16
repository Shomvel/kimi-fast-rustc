use super::*;
use crate::{
    ArithmeticFlags as F, GraphLevel, InterpretError, Interpreter, Outcome, SemanticOutcome, Value,
    ValueType, lower_layout, verify_graph,
};
use kfrc_body::{
    FunctionAbi, IntegerWidth, PassMode, RecordedTypes, ScalarValidRange, ScalarValidity, SizeAlign,
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
            "strength",
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
            .map(|index| projection(index as u32 + 2, ValueType::Rust(ty)))
            .collect();
        Self {
            graph,
            ty,
            args,
            control,
            effect,
        }
    }

    fn constant(&mut self, value: u64) -> NodeId {
        self.graph.add_constant(u128::from(value), self.ty)
    }

    fn binary(&mut self, op: Opcode, a: NodeId, b: NodeId, flags: u32) -> NodeId {
        self.graph
            .add_node(op, NodeId::NONE, &[a, b], flags, ValueType::Rust(self.ty))
    }

    fn pair(&mut self, first: NodeId, second: NodeId) -> NodeId {
        let ty = self
            .graph
            .oracle
            .intern(TypeShape::Tuple(vec![self.ty, self.ty]), None);
        self.graph.add_node(
            Opcode::Aggregate,
            NodeId::NONE,
            &[first, second],
            0,
            ValueType::Rust(ty),
        )
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

    fn rewrite(&mut self, node: NodeId) -> NodeId {
        let uses = Uses::new(&self.graph);
        let replacement = combine(&mut self.graph, node, &uses).expect("strength rule must match");
        self.graph.replace(node, replacement);
        self.graph.compact();
        verify_graph(&mut self.graph).unwrap();
        let ret = self
            .graph
            .ids()
            .find(|&node| self.graph.op(node) == Opcode::Return)
            .unwrap();
        self.graph.input(ret, 1)
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Failure {
    Poison,
    Undefined,
}

fn outcome(graph: &FunctionGraph, args: &[Value]) -> Result<SemanticOutcome, Failure> {
    Interpreter::new(graph)
        .run_values(args, &mut |_: &str, _: &[Value]| unreachable!())
        .map_err(|error| match error {
            InterpretError::PoisonObserved(_) => Failure::Poison,
            InterpretError::UndefinedBehaviour(_) => Failure::Undefined,
            other => panic!("unexpected interpreter error: {other:?}"),
        })
}

fn check_scalar(before: &FunctionGraph, after: &FunctionGraph, args: &[u64], expected: u64) {
    let args: Vec<_> = args.iter().copied().map(Value::Scalar).collect();
    let expected = Ok(SemanticOutcome::Return(Value::Scalar(expected)));
    assert_eq!(outcome(before, &args), expected);
    assert_eq!(outcome(after, &args), expected);
}

fn check_poison(before: &FunctionGraph, after: &FunctionGraph, args: &[Value]) {
    assert_eq!(outcome(before, args), Err(Failure::Poison));
    assert_eq!(outcome(after, args), Err(Failure::Poison));
}

fn mask(width: IntegerWidth) -> u64 {
    u64::MAX >> (64 - width.bits(64))
}

fn values(width: IntegerWidth) -> Vec<u64> {
    if width == IntegerWidth::W8 {
        return (0..=255).collect();
    }
    let mask = mask(width);
    vec![0, 1, 2, mask >> 1, (mask >> 1) + 1, mask - 1, mask]
}

#[test]
fn constant_power_of_two_products_match_wrapping_arithmetic_at_every_width() {
    for width in [
        IntegerWidth::W8,
        IntegerWidth::W16,
        IntegerWidth::W32,
        IntegerWidth::W64,
    ] {
        for signed in [false, true] {
            for shift in 1..width.bits(64) {
                for commuted in [false, true] {
                    let mut f = Function::new(width, signed, 1);
                    let factor = f.constant(1u64 << shift);
                    let source = f.args[0];
                    let (a, b) = if commuted {
                        (factor, source)
                    } else {
                        (source, factor)
                    };
                    let product = f.binary(Opcode::Mul, a, b, 0);
                    f.finish(product);
                    let before = f.graph.clone();
                    let result = f.rewrite(product);
                    assert_eq!(f.graph.op(result), Opcode::ShiftLeft);
                    for value in values(width) {
                        check_scalar(
                            &before,
                            &f.graph,
                            &[value],
                            value.wrapping_mul(1u64 << shift) & mask(width),
                        );
                    }
                    check_poison(&before, &f.graph, &[Value::Poison]);
                }
            }
        }
    }
}

#[test]
fn opposite_shifts_use_each_original_width_mask_before_cancelling() {
    for width in [
        IntegerWidth::W8,
        IntegerWidth::W16,
        IntegerWidth::W32,
        IntegerWidth::W64,
    ] {
        let bits = u64::from(width.bits(64));
        for signed in [false, true] {
            for inner_left in [false, true] {
                for (inner_amount, outer_amount) in [
                    (0, bits),
                    (1, 1),
                    (bits + 1, 1),
                    (1, bits + 1),
                    (bits - 1, 2 * bits - 1),
                ] {
                    let mut f = Function::new(width, signed, 1);
                    let inner_amount = f.constant(inner_amount);
                    let outer_amount = f.constant(outer_amount);
                    let (inner_op, outer_op) = if inner_left {
                        (Opcode::ShiftLeft, Opcode::ShiftRightLogical)
                    } else {
                        (Opcode::ShiftRightLogical, Opcode::ShiftLeft)
                    };
                    let inner = f.binary(inner_op, f.args[0], inner_amount, 0);
                    let outer = f.binary(outer_op, inner, outer_amount, 0);
                    f.finish(outer);
                    let amount = constant(&f.graph, inner_amount).unwrap() % bits;
                    let before = f.graph.clone();
                    let result = f.rewrite(outer);
                    assert_eq!(f.graph.op(result), Opcode::BitAnd);
                    for value in values(width) {
                        let expected = if inner_left {
                            ((value << amount) & mask(width)) >> amount
                        } else {
                            ((value >> amount) << amount) & mask(width)
                        };
                        check_scalar(&before, &f.graph, &[value], expected);
                    }
                    check_poison(&before, &f.graph, &[Value::Poison]);
                }
            }
        }
    }
}

#[test]
fn dynamic_shift_factors_keep_both_poison_inputs_and_masked_amounts() {
    for width in [
        IntegerWidth::W8,
        IntegerWidth::W16,
        IntegerWidth::W32,
        IntegerWidth::W64,
    ] {
        let bits = u64::from(width.bits(64));
        for signed in [false, true] {
            for commuted in [false, true] {
                let mut f = Function::new(width, signed, 2);
                let one = f.constant(1);
                let factor = f.binary(Opcode::ShiftLeft, one, f.args[1], 0);
                let (a, b) = if commuted {
                    (factor, f.args[0])
                } else {
                    (f.args[0], factor)
                };
                let product = f.binary(Opcode::Mul, a, b, 0);
                f.finish(product);
                let before = f.graph.clone();
                let result = f.rewrite(product);
                assert_eq!(f.graph.op(result), Opcode::ShiftLeft);
                let amounts = if width == IntegerWidth::W8 {
                    (0..=255).collect::<Vec<_>>()
                } else {
                    vec![0, 1, bits - 1, bits, bits + 1, 2 * bits + 1, mask(width)]
                };
                for value in values(width) {
                    for &amount in &amounts {
                        check_scalar(
                            &before,
                            &f.graph,
                            &[value, amount],
                            value.wrapping_shl((amount % bits) as u32) & mask(width),
                        );
                    }
                }
                for args in [
                    [Value::Poison, Value::Scalar(bits)],
                    [Value::Scalar(0), Value::Poison],
                    [Value::Poison, Value::Poison],
                ] {
                    check_poison(&before, &f.graph, &args);
                }
            }
        }
    }
}

#[test]
fn reductions_survive_layout_lowering_with_a_wider_shift_amount_type() {
    for variant in 0..3 {
        let mut f = Function::new(IntegerWidth::W8, false, 2);
        let amount_ty = f.graph.oracle.intern(
            TypeShape::Int {
                width: IntegerWidth::W64,
                signed: false,
            },
            Some(SizeAlign { size: 8, align: 8 }),
        );
        let ValueType::Tuple(mut outputs) = f.graph.value_type(f.graph.start).clone() else {
            unreachable!()
        };
        outputs[3] = ValueType::Rust(amount_ty);
        f.graph
            .set_value_type(f.graph.start, ValueType::Tuple(outputs));
        f.graph
            .set_value_type(f.args[1], ValueType::Rust(amount_ty));
        let candidate = match variant {
            0 => {
                let four = f.constant(4);
                f.binary(Opcode::Mul, f.args[0], four, 0)
            }
            1 => {
                let nine = f.graph.add_constant(9, amount_ty);
                let one = f.constant(1);
                let inner = f.binary(Opcode::ShiftLeft, f.args[0], nine, 0);
                f.binary(Opcode::ShiftRightLogical, inner, one, 0)
            }
            _ => {
                let one = f.constant(1);
                let factor = f.binary(Opcode::ShiftLeft, one, f.args[1], 0);
                f.binary(Opcode::Mul, f.args[0], factor, 0)
            }
        };
        f.finish(candidate);
        let before = f.graph.clone();
        f.rewrite(candidate);
        lower_layout(&mut f.graph).unwrap();
        verify_graph(&mut f.graph).unwrap();
        for value in [0, 1, 127, 128, 255] {
            for amount in [0, 8, 9, 255, 256, 257, u64::MAX] {
                let expected = outcome(&before, &[Value::Scalar(value), Value::Scalar(amount)]);
                let Ok(SemanticOutcome::Return(Value::Scalar(expected))) = expected else {
                    panic!("expected an ordinary scalar return")
                };
                assert_eq!(
                    Interpreter::new(&f.graph)
                        .run(&[value, amount], &mut |_: &str, _: &[u64]| unreachable!()),
                    Ok(Outcome::Return(expected)),
                );
            }
        }
    }
}

#[test]
fn constant_product_rewrite_preserves_a_source_overflow_contract_and_shared_result() {
    let mut f = Function::new(IntegerWidth::W8, false, 1);
    let limit = f.constant(128);
    let source = f.binary(Opcode::Add, f.args[0], limit, F::NUW);
    let four = f.constant(4);
    let product = f.binary(Opcode::Mul, source, four, 0);
    let one = f.constant(1);
    let adjusted = f.binary(Opcode::Add, product, one, 0);
    let result = f.pair(product, adjusted);
    f.finish(result);
    let before = f.graph.clone();
    f.rewrite(product);
    assert!(
        f.graph
            .ids()
            .any(|node| { f.graph.op(node) == Opcode::Add && f.graph.payload(node) == F::NUW })
    );
    for value in 0..=255 {
        let args = [Value::Scalar(value)];
        assert_eq!(outcome(&before, &args), outcome(&f.graph, &args));
    }
    check_poison(&before, &f.graph, &[Value::Poison]);
}

fn checked_flag(f: &mut Function, a: NodeId, b: NodeId) -> (NodeId, NodeId) {
    let boolean = f
        .graph
        .oracle
        .intern(TypeShape::Bool, Some(SizeAlign { size: 1, align: 1 }));
    let overflow = f.graph.add_node(
        Opcode::MulOverflow,
        NodeId::NONE,
        &[a, b],
        0,
        ValueType::Rust(boolean),
    );
    let flag = f.graph.add_node(
        Opcode::IntCast,
        NodeId::NONE,
        &[overflow],
        0,
        ValueType::Rust(f.ty),
    );
    (overflow, flag)
}

#[test]
fn checked_unsigned_word_products_keep_the_shared_multiply_and_both_results() {
    for width in [IntegerWidth::W64, IntegerWidth::Pointer] {
        for commuted_product in [false, true] {
            for commuted_check in [false, true] {
                for distinct_constant in [false, true] {
                    let mut f = Function::new(width, false, 1);
                    let factor = f.constant(4);
                    let checked_factor = if distinct_constant {
                        f.constant(4)
                    } else {
                        factor
                    };
                    assert_eq!(factor == checked_factor, !distinct_constant);
                    let source = f.args[0];
                    let (a, b) = if commuted_product {
                        (factor, source)
                    } else {
                        (source, factor)
                    };
                    let product = f.binary(Opcode::Mul, a, b, 0);
                    let (a, b) = if commuted_check {
                        (checked_factor, source)
                    } else {
                        (source, checked_factor)
                    };
                    let (_, flag) = checked_flag(&mut f, a, b);
                    let result = f.pair(product, flag);
                    f.finish(result);
                    let before = f.graph.clone();
                    let uses = Uses::new(&f.graph);
                    let dump = f.graph.dump();
                    assert!(combine(&mut f.graph, product, &uses).is_none());
                    assert_eq!(f.graph.dump(), dump);
                    assert_eq!(f.graph.op(product), Opcode::Mul);
                    assert_eq!(f.graph.payload(product), 0);
                    for value in [0, 1, u64::MAX / 4, u64::MAX / 4 + 1, u64::MAX] {
                        let (product, overflow) = value.overflowing_mul(4);
                        let expected = Ok(SemanticOutcome::Return(Value::Aggregate(vec![
                            Value::Scalar(product),
                            Value::Scalar(u64::from(overflow)),
                        ])));
                        assert_eq!(outcome(&before, &[Value::Scalar(value)]), expected);
                        assert_eq!(outcome(&f.graph, &[Value::Scalar(value)]), expected);
                    }
                    check_poison(&before, &f.graph, &[Value::Poison]);
                }
            }
        }
    }
}

#[test]
fn unrelated_or_unused_overflow_checks_do_not_block_power_of_two_reduction() {
    for variant in 0..7 {
        let (width, signed) = match variant {
            3 => (IntegerWidth::W32, false),
            4 => (IntegerWidth::W64, true),
            _ => (IntegerWidth::W64, false),
        };
        let mut f = Function::new(width, signed, 2);
        let four = f.constant(4);
        let eight = f.constant(8);
        let product = f.binary(Opcode::Mul, f.args[0], four, 0);
        let (left, right) = match variant {
            0 => (f.args[1], four),  // Different variable.
            1 => (f.args[0], eight), // Different constant.
            _ => (f.args[0], four),
        };
        let (overflow, flag) = checked_flag(&mut f, left, right);
        if variant == 5 {
            // A different ordinary type has no shared checked.rs key, even
            // when its constant bits and width happen to match.
            let other = f.graph.oracle.intern(
                TypeShape::Int {
                    width: IntegerWidth::Pointer,
                    signed: false,
                },
                Some(SizeAlign { size: 8, align: 8 }),
            );
            let left = f.graph.add_node(
                Opcode::IntCast,
                NodeId::NONE,
                &[f.args[0]],
                0,
                ValueType::Rust(other),
            );
            let right = f.graph.add_constant(4, other);
            f.graph.set_inputs(overflow, &[left, right]);
        } else if variant == 6 {
            // The same operands in an unrelated operation are not a check.
            let boolean = f.graph.rust_type(overflow).unwrap();
            let equal = f.graph.add_node(
                Opcode::Compare,
                NodeId::NONE,
                &[left, right],
                crate::ComparePredicate::Equal as u32,
                ValueType::Rust(boolean),
            );
            f.graph.set_input(flag, 0, equal);
        }
        let result = if variant == 2 {
            product
        } else {
            f.pair(product, flag)
        };
        f.finish(result);
        let before = f.graph.clone();
        f.rewrite(product);
        assert!(
            f.graph
                .ids()
                .any(|node| f.graph.op(node) == Opcode::ShiftLeft)
        );
        for a in [0, 1, 127, 128, mask(width) >> 2, mask(width)] {
            for b in [0, 1, mask(width)] {
                let args = [Value::Scalar(a), Value::Scalar(b)];
                assert_eq!(
                    outcome(&before, &args),
                    outcome(&f.graph, &args),
                    "variant={variant}, args={args:?}"
                );
            }
        }
    }
}

#[test]
fn checked_product_cost_guard_validates_current_edges_after_the_snapshot() {
    for stale_result in [false, true] {
        let mut f = Function::new(IntegerWidth::W64, false, 2);
        let four = f.constant(4);
        let source = f.args[0];
        let product = f.binary(Opcode::Mul, source, four, 0);
        let (overflow, flag) = checked_flag(&mut f, source, four);
        let result = f.pair(product, flag);
        f.finish(result);
        let uses = Uses::new(&f.graph);
        if stale_result {
            let zero = f.constant(0);
            f.graph.replace(flag, zero);
            // Replacing the cast leaves the overflow with a dead old user.
        } else {
            f.graph.set_input(overflow, 0, f.args[1]);
        }
        let replacement = combine(&mut f.graph, product, &uses)
            .expect("obsolete sharing must not block reduction");
        assert_eq!(f.graph.op(replacement), Opcode::ShiftLeft);
    }
}

#[test]
fn a_forwarded_checked_operand_is_found_through_its_snapshot_identity() {
    for resolve_edges in [false, true] {
        let mut f = Function::new(IntegerWidth::W64, false, 2);
        let four = f.constant(4);
        let source = f.args[0];
        let product = f.binary(Opcode::Mul, source, four, 0);
        let (_, flag) = checked_flag(&mut f, source, four);
        let result = f.pair(product, flag);
        f.finish(result);
        let uses = Uses::new(&f.graph);
        let one = f.constant(1);
        let forwarded = f.binary(Opcode::Add, f.args[1], one, 0);
        f.graph.replace(source, forwarded);
        if resolve_edges {
            f.graph.resolve_edges();
        }
        let dump = f.graph.dump();
        assert!(combine(&mut f.graph, product, &uses).is_none());
        assert_eq!(f.graph.dump(), dump);
    }
}

#[test]
fn shared_shift_producers_stay_for_their_full_value_observer() {
    for opposite in [false, true] {
        let mut f = Function::new(IntegerWidth::W8, false, 2);
        let one = f.constant(1);
        let (producer, candidate) = if opposite {
            let producer = f.binary(Opcode::ShiftLeft, f.args[0], one, 0);
            let candidate = f.binary(Opcode::ShiftRightLogical, producer, one, 0);
            (producer, candidate)
        } else {
            let producer = f.binary(Opcode::ShiftLeft, one, f.args[1], 0);
            let candidate = f.binary(Opcode::Mul, f.args[0], producer, 0);
            (producer, candidate)
        };
        let result = f.pair(candidate, producer);
        f.finish(result);
        let uses = Uses::new(&f.graph);
        let original = f.graph.dump();
        assert!(combine(&mut f.graph, candidate, &uses).is_none());
        assert_eq!(f.graph.dump(), original);
    }
}

#[test]
fn arithmetic_promises_on_the_replaced_operations_are_preserved_by_rejection() {
    for (op, flags, signed) in [
        (Opcode::Mul, F::NUW, false),
        (Opcode::Mul, F::NSW, true),
        (Opcode::ShiftLeft, F::NUW, false),
        (Opcode::ShiftLeft, F::NSW, true),
        (Opcode::ShiftLeft, F::AMOUNT_IN_RANGE, false),
        (Opcode::ShiftRightLogical, F::EXACT, false),
        (Opcode::ShiftRightLogical, F::AMOUNT_IN_RANGE, false),
    ] {
        for on_inner in [false, true] {
            let mut f = Function::new(IntegerWidth::W8, signed, 2);
            let one = f.constant(1);
            let candidate = if op == Opcode::Mul {
                let factor = f.constant(4);
                f.binary(Opcode::Mul, f.args[0], factor, flags)
            } else if on_inner {
                let inner = f.binary(op, f.args[0], one, flags);
                let outer = if op == Opcode::ShiftLeft {
                    Opcode::ShiftRightLogical
                } else {
                    Opcode::ShiftLeft
                };
                f.binary(outer, inner, one, 0)
            } else {
                let inner_op = if op == Opcode::ShiftLeft {
                    Opcode::ShiftRightLogical
                } else {
                    Opcode::ShiftLeft
                };
                let inner = f.binary(inner_op, f.args[0], one, 0);
                f.binary(op, inner, one, flags)
            };
            f.finish(candidate);
            let uses = Uses::new(&f.graph);
            let original = f.graph.dump();
            assert!(combine(&mut f.graph, candidate, &uses).is_none());
            assert_eq!(f.graph.dump(), original);
        }
    }
    for flags in [F::NUW, F::NSW, F::AMOUNT_IN_RANGE] {
        let mut f = Function::new(IntegerWidth::W8, flags == F::NSW, 2);
        let one = f.constant(1);
        let factor = f.binary(Opcode::ShiftLeft, one, f.args[1], flags);
        let candidate = f.binary(Opcode::Mul, f.args[0], factor, 0);
        f.finish(candidate);
        let uses = Uses::new(&f.graph);
        assert!(combine(&mut f.graph, candidate, &uses).is_none());
    }
}

#[test]
fn a_forwarded_input_added_after_the_use_snapshot_waits_for_the_next_sweep() {
    for opposite in [false, true] {
        let mut f = Function::new(IntegerWidth::W8, false, 2);
        let one = f.constant(1);
        let old = f.binary(Opcode::Add, f.args[0], one, 0);
        let candidate = f.binary(
            if opposite {
                Opcode::ShiftRightLogical
            } else {
                Opcode::Mul
            },
            old,
            one,
            0,
        );
        f.finish(candidate);
        let uses = Uses::new(&f.graph);
        let replacement = if opposite {
            f.binary(Opcode::ShiftLeft, f.args[0], one, 0)
        } else {
            f.binary(Opcode::ShiftLeft, one, f.args[1], 0)
        };
        f.graph.replace(old, replacement);
        if !opposite {
            f.graph.set_inputs(candidate, &[f.args[0], replacement]);
        }
        assert!(combine(&mut f.graph, candidate, &uses).is_none());
        let uses = Uses::new(&f.graph);
        assert!(combine(&mut f.graph, candidate, &uses).is_some());
    }
}

#[test]
fn unsupported_scalar_types_widths_and_anchored_operations_are_rejected() {
    for shape in [
        TypeShape::Bool,
        TypeShape::Char,
        TypeShape::Int {
            width: IntegerWidth::W128,
            signed: false,
        },
    ] {
        // Deliberately invalid S subsets exercise only the matcher boundary.
        let mut types = RecordedTypes::new();
        let ty = types.intern(shape, None);
        let mut graph = FunctionGraph::new(
            "unsupported_strength",
            FunctionAbi {
                args: vec![],
                ret: PassMode::Direct,
                c_variadic: false,
            },
            types,
        );
        let source = graph.add_node(Opcode::Undefined, NodeId::NONE, &[], 0, ValueType::Rust(ty));
        let four = graph.add_constant(4, ty);
        let candidate = graph.add_node(
            Opcode::Mul,
            NodeId::NONE,
            &[source, four],
            0,
            ValueType::Rust(ty),
        );
        let uses = Uses::new(&graph);
        let nodes = graph.len();
        assert!(combine(&mut graph, candidate, &uses).is_none());
        assert_eq!(graph.len(), nodes);
    }
    let mut f = Function::new(IntegerWidth::W8, false, 1);
    let four = f.constant(4);
    let candidate = f.graph.add_node(
        Opcode::Mul,
        f.control,
        &[f.args[0], four],
        0,
        ValueType::Rust(f.ty),
    );
    let uses = Uses::new(&f.graph);
    assert!(combine(&mut f.graph, candidate, &uses).is_none());

    let mut f = Function::new(IntegerWidth::W8, false, 1);
    f.graph.oracle.record_scalar_validity(
        f.ty,
        ScalarValidity::Scalar(ScalarValidRange::new(8, 1, 255).unwrap()),
    );
    let four = f.constant(4);
    let candidate = f.binary(Opcode::Mul, f.args[0], four, 0);
    let uses = Uses::new(&f.graph);
    let nodes = f.graph.len();
    assert!(combine(&mut f.graph, candidate, &uses).is_none());
    assert_eq!(f.graph.len(), nodes);
}
