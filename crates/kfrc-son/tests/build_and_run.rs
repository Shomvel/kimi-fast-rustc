//! Body → R graph → verifier → interpreter on hand-built scalar bodies.

use kfrc_body::*;
use kfrc_son::{
    BuildError, FunctionGraph, InterpretError, Interpreter, NodeId, Opcode, Outcome, ValueType,
    build_graph, verify::check, verify_graph,
};

struct Types {
    recorded: RecordedTypes,
    i64: TyId,
    u8: TyId,
    i8: TyId,
    bool: TyId,
    unit: TyId,
}

fn types() -> Types {
    let mut recorded = RecordedTypes::new();
    let i64 = recorded.intern(
        TypeShape::Int {
            width: IntegerWidth::W64,
            signed: true,
        },
        Some(SizeAlign { size: 8, align: 8 }),
    );
    let u8 = recorded.intern(
        TypeShape::Int {
            width: IntegerWidth::W8,
            signed: false,
        },
        Some(SizeAlign { size: 1, align: 1 }),
    );
    let i8 = recorded.intern(
        TypeShape::Int {
            width: IntegerWidth::W8,
            signed: true,
        },
        Some(SizeAlign { size: 1, align: 1 }),
    );
    let bool = recorded.intern(TypeShape::Bool, Some(SizeAlign { size: 1, align: 1 }));
    let unit = recorded.intern(TypeShape::Unit, Some(SizeAlign { size: 0, align: 1 }));
    Types {
        recorded,
        i64,
        u8,
        i8,
        bool,
        unit,
    }
}

fn local(builder: &mut BodyBuilder, ty: TyId) -> LocalId {
    builder.add_local(LocalDecl {
        ty,
        mutability: Mutability::Mut,
        name: None,
    })
}

fn copy(local: LocalId) -> Operand {
    Operand::Copy(Place::local(local))
}

fn constant(builder: &mut BodyBuilder, ty: TyId, bits: u128) -> Operand {
    Operand::Constant(builder.add_constant(Constant {
        ty,
        kind: ConstantKind::Scalar(bits),
    }))
}

fn assign(builder: &mut BodyBuilder, target: LocalId, rvalue: Rvalue) {
    builder.push_statement(StatementKind::Assign(Place::local(target), rvalue));
}

fn abi(args: usize) -> FunctionAbi {
    FunctionAbi {
        args: vec![PassMode::Direct; args],
        ret: PassMode::Direct,
        c_variadic: false,
    }
}

fn run(graph: &FunctionGraph, args: &[u64]) -> Outcome {
    let mut no_calls =
        |symbol: &str, _: &[u64]| Err(InterpretError::UnknownCallee(symbol.to_owned()));
    Interpreter::new(&layout_graph(graph))
        .run(args, &mut no_calls)
        .expect("interpretation succeeds")
}

fn build_verified(body: &Body) -> FunctionGraph {
    let mut graph = build_graph(body).unwrap_or_else(|error| panic!("{error}\n{body}"));
    verify_graph(&mut graph).unwrap_or_else(|error| panic!("{error}\n{}", graph.dump()));
    graph
}

/// The T02 running example: `let mut x = a; if f { x = x + 1; } x`.
#[test]
fn add_if_merges_branch_assignment() {
    let t = types();
    let mut builder = BodyBuilder::new("add_if", abi(2), t.recorded.clone());
    let ret = local(&mut builder, t.i64);
    let f = local(&mut builder, t.bool);
    let a = local(&mut builder, t.i64);
    builder.set_arg_count(2);
    let x = local(&mut builder, t.i64);
    assign(&mut builder, x, Rvalue::Use(copy(a)));
    builder.finish_block(
        Terminator::SwitchInt {
            discriminant: copy(f),
            targets: SwitchTargets {
                cases: vec![(0, BlockId(2))],
                otherwise: BlockId(1),
            },
        },
        false,
    );
    let one = constant(&mut builder, t.i64, 1);
    assign(
        &mut builder,
        x,
        Rvalue::BinaryOp(BinaryOp::Add, copy(x), one),
    );
    builder.finish_block(Terminator::Goto(BlockId(2)), false);
    assign(&mut builder, ret, Rvalue::Use(copy(x)));
    builder.finish_block(Terminator::Return, false);
    let graph = build_verified(&builder.finish());
    assert_eq!(run(&graph, &[1, 41]), Outcome::Return(42));
    assert_eq!(run(&graph, &[0, 41]), Outcome::Return(41));
    assert!(
        graph.ids().any(|node| graph.op(node) == Opcode::Phi),
        "{}",
        graph.dump()
    );
}

/// `let (mut i, mut s) = (0, 0); while i < n { s += i; i += 1 } s`
#[test]
fn while_loop_carries_two_variables() {
    let t = types();
    let mut builder = BodyBuilder::new("sum", abi(1), t.recorded.clone());
    let ret = local(&mut builder, t.i64);
    let n = local(&mut builder, t.i64);
    builder.set_arg_count(1);
    let i = local(&mut builder, t.i64);
    let s = local(&mut builder, t.i64);
    let c = local(&mut builder, t.bool);
    let zero = constant(&mut builder, t.i64, 0);
    assign(&mut builder, i, Rvalue::Use(zero));
    assign(&mut builder, s, Rvalue::Use(zero));
    builder.finish_block(Terminator::Goto(BlockId(1)), false);
    assign(
        &mut builder,
        c,
        Rvalue::BinaryOp(BinaryOp::Lt, copy(i), copy(n)),
    );
    builder.finish_block(
        Terminator::SwitchInt {
            discriminant: copy(c),
            targets: SwitchTargets {
                cases: vec![(0, BlockId(3))],
                otherwise: BlockId(2),
            },
        },
        false,
    );
    let one = constant(&mut builder, t.i64, 1);
    assign(
        &mut builder,
        s,
        Rvalue::BinaryOp(BinaryOp::Add, copy(s), copy(i)),
    );
    assign(
        &mut builder,
        i,
        Rvalue::BinaryOp(BinaryOp::Add, copy(i), one),
    );
    builder.finish_block(Terminator::Goto(BlockId(1)), false);
    assign(&mut builder, ret, Rvalue::Use(copy(s)));
    builder.finish_block(Terminator::Return, false);
    let graph = build_verified(&builder.finish());
    assert_eq!(run(&graph, &[10]), Outcome::Return(45));
    assert_eq!(run(&graph, &[0]), Outcome::Return(0));
}

/// `match x { 0 => 10, 7 => 20, _ => 30 }` over `u8`.
#[test]
fn multiway_switch() {
    let t = types();
    let mut builder = BodyBuilder::new("pick", abi(1), t.recorded.clone());
    let ret = local(&mut builder, t.i64);
    let x = local(&mut builder, t.u8);
    builder.set_arg_count(1);
    builder.finish_block(
        Terminator::SwitchInt {
            discriminant: copy(x),
            targets: SwitchTargets {
                cases: vec![(0, BlockId(1)), (7, BlockId(2))],
                otherwise: BlockId(3),
            },
        },
        false,
    );
    for value in [10, 20, 30] {
        let value = constant(&mut builder, t.i64, value);
        assign(&mut builder, ret, Rvalue::Use(value));
        builder.finish_block(Terminator::Goto(BlockId(4)), false);
    }
    builder.finish_block(Terminator::Return, false);
    let graph = build_verified(&builder.finish());
    assert_eq!(run(&graph, &[0]), Outcome::Return(10));
    assert_eq!(run(&graph, &[7]), Outcome::Return(20));
    assert_eq!(
        run(&graph, &[0x107]),
        Outcome::Return(20),
        "upper argument bits are ignored for u8"
    );
    assert_eq!(run(&graph, &[3]), Outcome::Return(30));
}

#[test]
fn narrow_integers_wrap_and_cast() {
    let t = types();
    let mut builder = BodyBuilder::new("narrow", abi(2), t.recorded.clone());
    let ret = local(&mut builder, t.i64);
    let a = local(&mut builder, t.u8);
    let b = local(&mut builder, t.i8);
    builder.set_arg_count(2);
    let sum = local(&mut builder, t.u8);
    let difference = local(&mut builder, t.i8);
    let wide_sum = local(&mut builder, t.i64);
    let wide_difference = local(&mut builder, t.i64);
    let one = constant(&mut builder, t.i8, 1);
    assign(
        &mut builder,
        sum,
        Rvalue::BinaryOp(BinaryOp::Add, copy(a), copy(a)),
    );
    assign(
        &mut builder,
        difference,
        Rvalue::BinaryOp(BinaryOp::Sub, copy(b), one),
    );
    assign(
        &mut builder,
        wide_sum,
        Rvalue::Cast(CastKind::IntToInt, copy(sum), t.i64),
    );
    assign(
        &mut builder,
        wide_difference,
        Rvalue::Cast(CastKind::IntToInt, copy(difference), t.i64),
    );
    let thousand = constant(&mut builder, t.i64, 1000);
    let scaled = local(&mut builder, t.i64);
    assign(
        &mut builder,
        scaled,
        Rvalue::BinaryOp(BinaryOp::Mul, copy(wide_sum), thousand),
    );
    assign(
        &mut builder,
        ret,
        Rvalue::BinaryOp(BinaryOp::Add, copy(scaled), copy(wide_difference)),
    );
    builder.finish_block(Terminator::Return, false);
    let graph = build_verified(&builder.finish());
    // (200 + 200) as u8 = 144; (-128i8 - 1) wraps to 127.
    assert_eq!(run(&graph, &[200, 0x80]), Outcome::Return(144 * 1000 + 127));
}

#[test]
fn calls_thread_the_effect_token() {
    let t = types();
    let mut builder = BodyBuilder::new("twice", abi(1), t.recorded.clone());
    let ret = local(&mut builder, t.i64);
    let x = local(&mut builder, t.i64);
    builder.set_arg_count(1);
    let first = local(&mut builder, t.i64);
    let callee = Callee::Function {
        symbol: "inc".to_owned(),
        abi: abi(1),
    };
    let callee = Operand::Constant(builder.add_constant(Constant {
        ty: t.unit,
        kind: ConstantKind::Function(callee),
    }));
    builder.finish_block(
        Terminator::Call {
            callee,
            args: vec![copy(x)],
            destination: Place::local(first),
            target: Some(BlockId(1)),
            unwind: UnwindAction::Unreachable,
        },
        false,
    );
    builder.finish_block(
        Terminator::Call {
            callee,
            args: vec![copy(first)],
            destination: Place::local(ret),
            target: Some(BlockId(2)),
            unwind: UnwindAction::Unreachable,
        },
        false,
    );
    builder.finish_block(Terminator::Return, false);
    let graph = build_verified(&builder.finish());
    let mut log = Vec::new();
    let mut handler = |symbol: &str, args: &[u64]| {
        log.push((symbol.to_owned(), args.to_vec()));
        Ok(args[0] + 1)
    };
    let outcome = Interpreter::new(&layout_graph(&graph))
        .run(&[5], &mut handler)
        .unwrap();
    assert_eq!(outcome, Outcome::Return(7));
    assert_eq!(
        log,
        vec![("inc".to_owned(), vec![5]), ("inc".to_owned(), vec![6])]
    );
}

#[test]
fn assert_terminator_is_reported_unsupported() {
    let t = types();
    let mut builder = BodyBuilder::new("checked", abi(1), t.recorded.clone());
    let _ret = local(&mut builder, t.i64);
    let x = local(&mut builder, t.bool);
    builder.set_arg_count(1);
    builder.finish_block(
        Terminator::Assert {
            condition: copy(x),
            expected: true,
            message: AssertKind::DivisionByZero,
            target: BlockId(1),
            unwind: UnwindAction::Unreachable,
        },
        false,
    );
    builder.finish_block(Terminator::Return, false);
    match build_graph(&builder.finish()) {
        Err(BuildError::Unsupported { what, .. }) => assert!(what.contains("Assert"), "{what}"),
        other => panic!("expected unsupported, got {other:?}"),
    }
}

/// The T01 miscompile shape built by hand: a value defined in one branch is
/// used after the merge without a phi. The verifier must reject it.
#[test]
fn verifier_rejects_branch_value_used_after_merge() {
    let t = types();
    let mut graph = FunctionGraph::new("bad", abi(2), t.recorded.clone());
    let start = graph.add_node(
        Opcode::Start,
        NodeId::NONE,
        &[],
        0,
        ValueType::Tuple(vec![
            ValueType::Control,
            ValueType::Effect,
            ValueType::Rust(t.bool),
            ValueType::Rust(t.i64),
        ]),
    );
    graph.start = start;
    let control = graph.add_node(
        Opcode::Projection,
        NodeId::NONE,
        &[start],
        0,
        ValueType::Control,
    );
    let effect = graph.add_node(
        Opcode::Projection,
        NodeId::NONE,
        &[start],
        1,
        ValueType::Effect,
    );
    let flag = graph.add_node(
        Opcode::Projection,
        NodeId::NONE,
        &[start],
        2,
        ValueType::Rust(t.bool),
    );
    let a = graph.add_node(
        Opcode::Projection,
        NodeId::NONE,
        &[start],
        3,
        ValueType::Rust(t.i64),
    );
    let branch = graph.add_node(
        Opcode::If,
        control,
        &[flag],
        0,
        ValueType::Tuple(vec![ValueType::Control, ValueType::Control]),
    );
    let taken = graph.add_node(
        Opcode::Projection,
        NodeId::NONE,
        &[branch],
        0,
        ValueType::Control,
    );
    let skipped = graph.add_node(
        Opcode::Projection,
        NodeId::NONE,
        &[branch],
        1,
        ValueType::Control,
    );
    let zero = graph.add_constant(0, t.i64);
    // A trapping division fixed in the taken branch.
    let quotient = graph.add_node(Opcode::DivSigned, taken, &[a, a], 0, ValueType::Rust(t.i64));
    let jump_taken = graph.add_node(Opcode::Jump, taken, &[], 0, ValueType::Control);
    let jump_skipped = graph.add_node(Opcode::Jump, skipped, &[], 0, ValueType::Control);
    let merge = graph.add_growable_node(Opcode::Region, NodeId::NONE, 2, 0, ValueType::Control);
    graph.set_input(merge, 0, jump_taken);
    graph.set_input(merge, 1, jump_skipped);
    let sum = graph.add_node(
        Opcode::Add,
        NodeId::NONE,
        &[quotient, zero],
        0,
        ValueType::Rust(t.i64),
    );
    graph.add_node(Opcode::Return, merge, &[effect, sum], 0, ValueType::Control);
    match check(&graph) {
        Err(kfrc_son::VerifyError::DominanceViolation { .. }) => {}
        other => panic!(
            "expected a dominance violation, got {other:?}\n{}",
            graph.dump()
        ),
    }
}

/// A borrow and direct writes must observe the same storage across a merge.
#[test]
fn borrowed_local_observes_direct_and_indirect_writes() {
    let mut t = types();
    let pointer = t.recorded.intern(
        TypeShape::RawPointer {
            mutability: Mutability::Mut,
            pointee: t.i64,
        },
        Some(SizeAlign { size: 8, align: 8 }),
    );
    let mut builder = BodyBuilder::new("alias", abi(2), t.recorded);
    let ret = local(&mut builder, t.i64);
    let value = local(&mut builder, t.i64);
    let flag = local(&mut builder, t.bool);
    builder.set_arg_count(2);
    let ptr = local(&mut builder, pointer);
    assign(
        &mut builder,
        ptr,
        Rvalue::RawPointer(Mutability::Mut, Place::local(value)),
    );
    builder.finish_block(
        Terminator::SwitchInt {
            discriminant: copy(flag),
            targets: SwitchTargets {
                cases: vec![(0, BlockId(2))],
                otherwise: BlockId(1),
            },
        },
        false,
    );
    let ten = constant(&mut builder, t.i64, 10);
    assign(
        &mut builder,
        value,
        Rvalue::BinaryOp(BinaryOp::Add, copy(value), ten),
    );
    builder.finish_block(Terminator::Goto(BlockId(3)), false);
    let projection = builder.add_projection(vec![ProjectionElem::Deref]);
    let indirect = Place {
        local: ptr,
        projection,
    };
    let twenty = constant(&mut builder, t.i64, 20);
    builder.push_statement(StatementKind::Assign(indirect, Rvalue::Use(twenty)));
    builder.finish_block(Terminator::Goto(BlockId(3)), false);
    assign(&mut builder, ret, Rvalue::Use(Operand::Copy(indirect)));
    builder.finish_block(Terminator::Return, false);
    let graph = build_verified(&builder.finish());
    assert_eq!(run(&graph, &[5, 1]), Outcome::Return(15));
    assert_eq!(run(&graph, &[5, 0]), Outcome::Return(20));
}

/// The layout is deliberately reordered: field order must not imply offsets.
#[test]
fn aggregates_use_recorded_field_offsets_and_copy_values() {
    let mut t = types();
    let tuple = t.recorded.intern(
        TypeShape::Tuple(vec![t.u8, t.i64]),
        Some(SizeAlign { size: 16, align: 8 }),
    );
    t.recorded.record_aggregate(
        tuple,
        AggregateLayout {
            fields: vec![
                FieldLayout {
                    ty: t.u8,
                    offset: 8,
                },
                FieldLayout {
                    ty: t.i64,
                    offset: 0,
                },
            ],
            variants: vec![],
        },
    );
    let mut builder = BodyBuilder::new("aggregate", abi(1), t.recorded);
    let ret = local(&mut builder, t.i64);
    let input = local(&mut builder, t.i64);
    builder.set_arg_count(1);
    let first = local(&mut builder, tuple);
    let second = local(&mut builder, tuple);
    let tag = constant(&mut builder, t.u8, 200);
    assign(
        &mut builder,
        first,
        Rvalue::Aggregate(AggregateKind::Tuple, vec![tag, copy(input)]),
    );
    assign(&mut builder, second, Rvalue::Use(copy(first)));
    let field = builder.add_projection(vec![ProjectionElem::Field(FieldIndex(1), t.i64)]);
    let changed = constant(&mut builder, t.i64, 999);
    builder.push_statement(StatementKind::Assign(
        Place {
            local: first,
            projection: field,
        },
        Rvalue::Use(changed),
    ));
    assign(
        &mut builder,
        ret,
        Rvalue::Use(Operand::Copy(Place {
            local: second,
            projection: field,
        })),
    );
    builder.finish_block(Terminator::Return, false);
    let graph = build_verified(&builder.finish());
    assert_eq!(run(&graph, &[42]), Outcome::Return(42));
}

#[test]
fn checked_arithmetic_reports_overflow_at_each_width() {
    for signed in [false, true] {
        for bits in [8, 16, 32, 64] {
            for op in [
                BinaryOp::AddWithOverflow,
                BinaryOp::SubWithOverflow,
                BinaryOp::MulWithOverflow,
            ] {
                let mut t = types();
                let width = match bits {
                    8 => IntegerWidth::W8,
                    16 => IntegerWidth::W16,
                    32 => IntegerWidth::W32,
                    _ => IntegerWidth::W64,
                };
                let integer = t.recorded.intern(
                    TypeShape::Int { width, signed },
                    Some(SizeAlign {
                        size: bits / 8,
                        align: bits / 8,
                    }),
                );
                let pair = t.recorded.intern(
                    TypeShape::Tuple(vec![integer, t.bool]),
                    Some(SizeAlign {
                        size: bits / 4,
                        align: bits / 8,
                    }),
                );
                t.recorded.record_aggregate(
                    pair,
                    AggregateLayout {
                        fields: vec![
                            FieldLayout {
                                ty: integer,
                                offset: 0,
                            },
                            FieldLayout {
                                ty: t.bool,
                                offset: bits / 8,
                            },
                        ],
                        variants: vec![],
                    },
                );
                let mut builder = BodyBuilder::new("overflow", abi(2), t.recorded);
                let ret = local(&mut builder, t.bool);
                let a = local(&mut builder, integer);
                let b = local(&mut builder, integer);
                builder.set_arg_count(2);
                let result = local(&mut builder, pair);
                assign(&mut builder, result, Rvalue::BinaryOp(op, copy(a), copy(b)));
                let flag =
                    builder.add_projection(vec![ProjectionElem::Field(FieldIndex(1), t.bool)]);
                assign(
                    &mut builder,
                    ret,
                    Rvalue::Use(Operand::Copy(Place {
                        local: result,
                        projection: flag,
                    })),
                );
                builder.finish_block(Terminator::Return, false);
                let graph = build_verified(&builder.finish());
                let modulus = 1u128 << bits;
                let mask = modulus - 1;
                for a in [0, 1, 2, (modulus / 2) - 1, modulus / 2, mask] {
                    for b in [0, 1, 2, (modulus / 2) - 1, modulus / 2, mask] {
                        let expected = if signed {
                            let as_signed = |value: u128| {
                                if value >= modulus / 2 {
                                    value as i128 - modulus as i128
                                } else {
                                    value as i128
                                }
                            };
                            let (a, b) = (as_signed(a), as_signed(b));
                            let full = match op {
                                BinaryOp::AddWithOverflow => a + b,
                                BinaryOp::SubWithOverflow => a - b,
                                _ => a * b,
                            };
                            full < -(modulus as i128 / 2) || full >= modulus as i128 / 2
                        } else {
                            match op {
                                BinaryOp::AddWithOverflow => a + b > mask,
                                BinaryOp::SubWithOverflow => a < b,
                                _ => a * b > mask,
                            }
                        };
                        assert_eq!(
                            run(&graph, &[a as u64, b as u64]),
                            Outcome::Return(u64::from(expected)),
                            "{op:?} signed={signed} bits={bits} a={a} b={b}"
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn pair_arguments_and_return_follow_physical_abi_lane_order() {
    let mut t = types();
    let pair = t.recorded.intern(
        TypeShape::Tuple(vec![t.u8, t.i64]),
        Some(SizeAlign { size: 16, align: 8 }),
    );
    let byte = FieldLayout {
        ty: t.u8,
        offset: 8,
    };
    let wide = FieldLayout {
        ty: t.i64,
        offset: 0,
    };
    t.recorded.record_aggregate(
        pair,
        AggregateLayout {
            fields: vec![byte, wide],
            variants: vec![],
        },
    );
    t.recorded.record_abi_pair(pair, [wide, byte]);
    let mut builder = BodyBuilder::new(
        "identity_pair",
        FunctionAbi {
            args: vec![PassMode::Pair],
            ret: PassMode::Pair,
            c_variadic: false,
        },
        t.recorded,
    );
    let ret = local(&mut builder, pair);
    let arg = local(&mut builder, pair);
    builder.set_arg_count(1);
    assign(&mut builder, ret, Rvalue::Use(copy(arg)));
    builder.finish_block(Terminator::Return, false);
    let graph = build_verified(&builder.finish());
    assert_eq!(run(&graph, &[42, 200]), Outcome::ReturnPair(42, 200));
}

#[test]
fn indirect_call_resolves_the_function_value() {
    let mut t = types();
    let pointer = t.recorded.intern(
        TypeShape::RawPointer {
            mutability: Mutability::Not,
            pointee: t.unit,
        },
        Some(SizeAlign { size: 8, align: 8 }),
    );
    let mut builder = BodyBuilder::new("indirect", abi(1), t.recorded);
    let ret = local(&mut builder, t.i64);
    let arg = local(&mut builder, t.i64);
    builder.set_arg_count(1);
    let target = Operand::Constant(builder.add_constant(Constant {
        ty: pointer,
        kind: ConstantKind::Address {
            symbol: "callee".to_owned(),
            addend: 0,
        },
    }));
    builder.finish_block(
        Terminator::CallIndirect {
            function: target,
            abi: abi(1),
            args: vec![copy(arg)],
            destination: Place::local(ret),
            target: Some(BlockId(1)),
            unwind: UnwindAction::Unreachable,
        },
        false,
    );
    builder.finish_block(Terminator::Return, false);
    let graph = build_verified(&builder.finish());
    let mut calls = |symbol: &str, args: &[u64]| {
        assert_eq!(symbol, "callee");
        Ok(args[0] + 3)
    };
    assert_eq!(
        Interpreter::new(&layout_graph(&graph))
            .run(&[39], &mut calls)
            .unwrap(),
        Outcome::Return(42)
    );
}

#[test]
fn enum_direct_niche_and_single_discriminants_use_their_encoding() {
    for encoding in [
        TagEncoding::Direct {
            discriminants: vec![7, 255],
        },
        TagEncoding::Niche {
            untagged_variant: 0,
            niche_variants: 1..=2,
            niche_start: 255,
        },
        TagEncoding::Single {
            variant: 2,
            discriminant: 17,
        },
    ] {
        let mut t = types();
        let en = t.recorded.intern(
            TypeShape::Adt {
                name: format!("Test{encoding:?}"),
            },
            Some(SizeAlign { size: 1, align: 1 }),
        );
        t.recorded.record_aggregate(
            en,
            AggregateLayout {
                fields: vec![],
                variants: vec![
                    vec![FieldLayout {
                        ty: t.u8,
                        offset: 0,
                    }],
                    vec![],
                    vec![],
                ],
            },
        );
        let tag_ty = if matches!(encoding, TagEncoding::Direct { .. }) {
            t.i8
        } else {
            t.u8
        };
        t.recorded.record_semantic_enum(
            en,
            match &encoding {
                TagEncoding::Direct { .. } => vec![7, u128::MAX, 2],
                TagEncoding::Niche { .. } => vec![0, 1, 2],
                TagEncoding::Single { .. } => vec![0, 1, 17],
                _ => unreachable!(),
            },
        );
        t.recorded.enums.insert(
            en,
            EnumLayout {
                tag: Some(FieldLayout {
                    ty: tag_ty,
                    offset: 0,
                }),
                encoding: encoding.clone(),
            },
        );
        let mut builder = BodyBuilder::new("enum_discriminant", abi(1), t.recorded);
        let ret = local(&mut builder, t.i64);
        let arg = local(&mut builder, t.u8);
        builder.set_arg_count(1);
        let value = local(&mut builder, en);
        let (variant, operands, expected) = match encoding {
            TagEncoding::Direct { .. } => (1, vec![], u64::MAX),
            TagEncoding::Niche { .. } => (2, vec![], 2),
            TagEncoding::Single { .. } => (2, vec![], 17),
            _ => unreachable!(),
        };
        let _ = arg;
        assign(
            &mut builder,
            value,
            Rvalue::Aggregate(
                AggregateKind::Adt {
                    ty: en,
                    variant: VariantIndex(variant),
                },
                operands,
            ),
        );
        assign(&mut builder, ret, Rvalue::Discriminant(Place::local(value)));
        builder.finish_block(Terminator::Return, false);
        let graph = build_verified(&builder.finish());
        assert_eq!(run(&graph, &[123]), Outcome::Return(expected));
    }
}

#[test]
fn repeated_array_copy_nonoverlapping_preserves_memory_effect_order() {
    let mut t = types();
    let usize_ty = t.recorded.intern(
        TypeShape::Int {
            width: IntegerWidth::Pointer,
            signed: false,
        },
        Some(SizeAlign { size: 8, align: 8 }),
    );
    let array = t.recorded.intern(
        TypeShape::Array {
            element: t.i64,
            length: 3,
        },
        Some(SizeAlign { size: 24, align: 8 }),
    );
    let pointer = t.recorded.intern(
        TypeShape::RawPointer {
            mutability: Mutability::Mut,
            pointee: t.i64,
        },
        Some(SizeAlign { size: 8, align: 8 }),
    );
    let mut builder = BodyBuilder::new("memory_copy", abi(1), t.recorded);
    let ret = local(&mut builder, t.i64);
    let value = local(&mut builder, t.i64);
    builder.set_arg_count(1);
    let source = local(&mut builder, array);
    let destination = local(&mut builder, array);
    let src_pointer = local(&mut builder, pointer);
    let dst_pointer = local(&mut builder, pointer);
    assign(&mut builder, source, Rvalue::Repeat(copy(value), 3));
    let first = builder.add_projection(vec![ProjectionElem::ConstantIndex {
        offset: 0,
        min_length: 3,
        from_end: false,
    }]);
    assign(
        &mut builder,
        src_pointer,
        Rvalue::RawPointer(
            Mutability::Not,
            Place {
                local: source,
                projection: first,
            },
        ),
    );
    assign(
        &mut builder,
        dst_pointer,
        Rvalue::RawPointer(
            Mutability::Mut,
            Place {
                local: destination,
                projection: first,
            },
        ),
    );
    let count = constant(&mut builder, usize_ty, 3);
    builder.push_statement(StatementKind::CopyNonOverlapping {
        src: copy(src_pointer),
        dst: copy(dst_pointer),
        count,
        element_size: 8,
    });
    let last = builder.add_projection(vec![ProjectionElem::ConstantIndex {
        offset: 1,
        min_length: 3,
        from_end: true,
    }]);
    assign(
        &mut builder,
        ret,
        Rvalue::Use(Operand::Copy(Place {
            local: destination,
            projection: last,
        })),
    );
    builder.finish_block(Terminator::Return, false);
    let graph = build_verified(&builder.finish());
    assert_eq!(run(&graph, &[42]), Outcome::Return(42));
}

#[test]
fn aggregate_transmute_reads_representation_bytes_instead_of_its_address() {
    let mut t = types();
    let array = t.recorded.intern(
        TypeShape::Array {
            element: t.u8,
            length: 8,
        },
        Some(SizeAlign { size: 8, align: 1 }),
    );
    let identifier = t.recorded.intern(
        TypeShape::Adt {
            name: "Identifier".to_owned(),
        },
        Some(SizeAlign { size: 8, align: 8 }),
    );
    t.recorded.record_aggregate(
        identifier,
        AggregateLayout {
            fields: vec![FieldLayout {
                ty: t.i64,
                offset: 0,
            }],
            variants: vec![],
        },
    );
    let mut builder = BodyBuilder::new("transmute_identifier", abi(0), t.recorded);
    let ret = local(&mut builder, t.i64);
    let bytes = local(&mut builder, array);
    let ident = local(&mut builder, identifier);
    let roundtrip = local(&mut builder, array);
    let constants = (1..=8)
        .map(|byte| constant(&mut builder, t.u8, byte))
        .collect();
    assign(
        &mut builder,
        bytes,
        Rvalue::Aggregate(AggregateKind::Array(t.u8), constants),
    );
    assign(
        &mut builder,
        ident,
        Rvalue::Cast(CastKind::Transmute, copy(bytes), identifier),
    );
    assign(
        &mut builder,
        roundtrip,
        Rvalue::Cast(CastKind::Transmute, copy(ident), array),
    );
    assign(
        &mut builder,
        ret,
        Rvalue::Cast(CastKind::Transmute, copy(roundtrip), t.i64),
    );
    builder.finish_block(Terminator::Return, false);
    let graph = build_verified(&builder.finish());
    assert_eq!(run(&graph, &[]), Outcome::Return(0x0807060504030201));
}

#[test]
fn anchored_division_value_is_available_on_its_control_continuation() {
    let t = types();
    let mut builder = BodyBuilder::new("division", abi(2), t.recorded);
    let ret = local(&mut builder, t.i64);
    let lhs = local(&mut builder, t.i64);
    let rhs = local(&mut builder, t.i64);
    builder.set_arg_count(2);
    assign(
        &mut builder,
        ret,
        Rvalue::BinaryOp(BinaryOp::Div, copy(lhs), copy(rhs)),
    );
    builder.finish_block(Terminator::Return, false);
    let graph = build_verified(&builder.finish());
    assert_eq!(run(&graph, &[84, 2]), Outcome::Return(42));
}

#[test]
fn aggregate_self_assignment_reads_every_rhs_field_before_writing() {
    let mut t = types();
    let pair = t.recorded.intern(
        TypeShape::Tuple(vec![t.i64, t.i64]),
        Some(SizeAlign { size: 16, align: 8 }),
    );
    t.recorded.record_aggregate(
        pair,
        AggregateLayout {
            fields: vec![
                FieldLayout {
                    ty: t.i64,
                    offset: 0,
                },
                FieldLayout {
                    ty: t.i64,
                    offset: 8,
                },
            ],
            variants: vec![],
        },
    );
    let mut builder = BodyBuilder::new("swap", abi(0), t.recorded);
    let ret = local(&mut builder, t.i64);
    let pair_local = local(&mut builder, pair);
    let first = builder.add_projection(vec![ProjectionElem::Field(FieldIndex(0), t.i64)]);
    let second = builder.add_projection(vec![ProjectionElem::Field(FieldIndex(1), t.i64)]);
    let a = constant(&mut builder, t.i64, 4);
    let b = constant(&mut builder, t.i64, 2);
    assign(
        &mut builder,
        pair_local,
        Rvalue::Aggregate(AggregateKind::Tuple, vec![a, b]),
    );
    assign(
        &mut builder,
        pair_local,
        Rvalue::Aggregate(
            AggregateKind::Tuple,
            vec![
                Operand::Copy(Place {
                    local: pair_local,
                    projection: second,
                }),
                Operand::Copy(Place {
                    local: pair_local,
                    projection: first,
                }),
            ],
        ),
    );
    let lhs = local(&mut builder, t.i64);
    let ten = constant(&mut builder, t.i64, 10);
    assign(
        &mut builder,
        lhs,
        Rvalue::BinaryOp(
            BinaryOp::Mul,
            Operand::Copy(Place {
                local: pair_local,
                projection: first,
            }),
            ten,
        ),
    );
    assign(
        &mut builder,
        ret,
        Rvalue::BinaryOp(
            BinaryOp::Add,
            copy(lhs),
            Operand::Copy(Place {
                local: pair_local,
                projection: second,
            }),
        ),
    );
    builder.finish_block(Terminator::Return, false);
    let graph = build_verified(&builder.finish());
    assert_eq!(run(&graph, &[]), Outcome::Return(24));
}

#[test]
fn direct_aggregate_abi_and_immediate_constants_preserve_bits() {
    let mut t = types();
    let wrapper = t.recorded.intern(
        TypeShape::Adt {
            name: "Wrapper".to_owned(),
        },
        Some(SizeAlign { size: 8, align: 8 }),
    );
    t.recorded.record_aggregate(
        wrapper,
        AggregateLayout {
            fields: vec![FieldLayout {
                ty: t.i64,
                offset: 0,
            }],
            variants: vec![],
        },
    );
    let mut builder = BodyBuilder::new("wrapper", abi(1), t.recorded);
    let ret = local(&mut builder, wrapper);
    let arg = local(&mut builder, wrapper);
    builder.set_arg_count(1);
    let ignored = constant(&mut builder, wrapper, 0x12345678);
    assign(&mut builder, ret, Rvalue::Use(ignored));
    let field = builder.add_projection(vec![ProjectionElem::Field(FieldIndex(0), t.i64)]);
    let return_field = Place {
        local: ret,
        projection: field,
    };
    let argument_field = Place {
        local: arg,
        projection: field,
    };
    builder.push_statement(StatementKind::Assign(
        return_field,
        Rvalue::BinaryOp(
            BinaryOp::Add,
            Operand::Copy(return_field),
            Operand::Copy(argument_field),
        ),
    ));
    builder.finish_block(Terminator::Return, false);
    let graph = build_verified(&builder.finish());
    assert_eq!(run(&graph, &[42]), Outcome::Return(0x12345678 + 42));
}

#[test]
fn fat_pointer_cast_reads_data_lane_and_metadata_is_preserved() {
    let mut t = types();
    let usize_ty = t.recorded.intern(
        TypeShape::Int {
            width: IntegerWidth::Pointer,
            signed: false,
        },
        Some(SizeAlign { size: 8, align: 8 }),
    );
    let slice = t.recorded.intern(TypeShape::Slice { element: t.u8 }, None);
    let thin = t.recorded.intern(
        TypeShape::RawPointer {
            mutability: Mutability::Not,
            pointee: t.u8,
        },
        Some(SizeAlign { size: 8, align: 8 }),
    );
    let fat = t.recorded.intern(
        TypeShape::Reference {
            mutability: Mutability::Not,
            pointee: slice,
        },
        Some(SizeAlign { size: 16, align: 8 }),
    );
    t.recorded.record_aggregate(
        fat,
        AggregateLayout {
            fields: vec![
                FieldLayout {
                    ty: thin,
                    offset: 0,
                },
                FieldLayout {
                    ty: usize_ty,
                    offset: 8,
                },
            ],
            variants: vec![],
        },
    );
    let mut builder = BodyBuilder::new("fat", abi(0), t.recorded);
    let ret = local(&mut builder, usize_ty);
    let fat_local = local(&mut builder, fat);
    let pointer = local(&mut builder, thin);
    let length = local(&mut builder, usize_ty);
    let byte = local(&mut builder, usize_ty);
    builder.add_data_object(DataObject {
        symbol: "bytes".to_owned(),
        binding: DataBinding::Local,
        bytes: vec![12, 34, 56],
        align: 1,
        writable: false,
        thread_local: false,
        relocations: vec![],
    });
    let data = Operand::Constant(builder.add_constant(Constant {
        ty: thin,
        kind: ConstantKind::Address {
            symbol: "bytes".to_owned(),
            addend: 0,
        },
    }));
    let count = constant(&mut builder, usize_ty, 3);
    assign(
        &mut builder,
        fat_local,
        Rvalue::Aggregate(AggregateKind::Tuple, vec![data, count]),
    );
    assign(
        &mut builder,
        length,
        Rvalue::UnaryOp(UnaryOp::PtrMetadata, copy(fat_local)),
    );
    assign(
        &mut builder,
        pointer,
        Rvalue::Cast(CastKind::PtrToPtr, copy(fat_local), thin),
    );
    let deref = builder.add_projection(vec![ProjectionElem::Deref]);
    assign(
        &mut builder,
        byte,
        Rvalue::Cast(
            CastKind::IntToInt,
            Operand::Copy(Place {
                local: pointer,
                projection: deref,
            }),
            usize_ty,
        ),
    );
    assign(
        &mut builder,
        ret,
        Rvalue::BinaryOp(BinaryOp::Add, copy(length), copy(byte)),
    );
    builder.finish_block(Terminator::Return, false);
    let graph = build_verified(&builder.finish());
    assert_eq!(run(&graph, &[]), Outcome::Return(15));
}

#[test]
fn opaque_value_remains_a_control_and_effect_barrier() {
    let t = types();
    let mut builder = BodyBuilder::new("opaque", abi(0), t.recorded);
    let ret = local(&mut builder, t.i64);
    let value = constant(&mut builder, t.i64, 42);
    assign(&mut builder, ret, Rvalue::Opaque(value));
    builder.finish_block(Terminator::Return, false);
    let mut graph = build_verified(&builder.finish());
    kfrc_son::optimize_graph(&mut graph);
    verify_graph(&mut graph).unwrap();
    assert!(graph.ids().any(|node| graph.op(node) == Opcode::Opaque));
    assert_eq!(run(&graph, &[]), Outcome::Return(42));
}

#[test]
fn three_way_comparison_constructs_recorded_ordering_tag() {
    for signed in [false, true] {
        let mut t = types();
        let operand_ty = if signed { t.i8 } else { t.u8 };
        let ordering = t.recorded.intern(
            TypeShape::Adt {
                name: "core::cmp::Ordering#producer-nominal-identity".to_owned(),
            },
            Some(SizeAlign { size: 8, align: 1 }),
        );
        t.recorded.record_aggregate(
            ordering,
            AggregateLayout {
                fields: vec![],
                variants: vec![vec![], vec![], vec![]],
            },
        );
        t.recorded
            .record_semantic_enum(ordering, vec![u128::MAX, 0, 1]);
        t.recorded.record_ordering(ordering);
        t.recorded.enums.insert(
            ordering,
            EnumLayout {
                tag: Some(FieldLayout {
                    ty: t.i8,
                    offset: 4,
                }),
                encoding: TagEncoding::Direct {
                    discriminants: vec![u128::MAX, 0, 1],
                },
            },
        );
        let mut builder = BodyBuilder::new("compare", abi(2), t.recorded);
        let ret = local(&mut builder, t.i64);
        let lhs = local(&mut builder, operand_ty);
        let rhs = local(&mut builder, operand_ty);
        builder.set_arg_count(2);
        let result = local(&mut builder, ordering);
        assign(
            &mut builder,
            result,
            Rvalue::BinaryOp(BinaryOp::Cmp, copy(lhs), copy(rhs)),
        );
        assign(
            &mut builder,
            ret,
            Rvalue::Discriminant(Place::local(result)),
        );
        builder.finish_block(Terminator::Return, false);
        let graph = build_verified(&builder.finish());
        for a in [0u8, 1, 127, 128, 255] {
            for b in [0u8, 1, 127, 128, 255] {
                let expected = if signed {
                    (a as i8).cmp(&(b as i8))
                } else {
                    a.cmp(&b)
                };
                let expected = match expected {
                    std::cmp::Ordering::Less => u64::MAX,
                    std::cmp::Ordering::Equal => 0,
                    std::cmp::Ordering::Greater => 1,
                };
                assert_eq!(
                    run(&graph, &[u64::from(a), u64::from(b)]),
                    Outcome::Return(expected),
                    "signed={signed},a={a},b={b}"
                );
            }
        }
    }
}

#[test]
fn three_way_comparison_requires_producer_ordering_identity_and_shape() {
    for (marked, discriminants, reason) in [
        (false, vec![u128::MAX, 0, 1], "semantic core::cmp::Ordering"),
        (true, vec![0, 1, 2], "source discriminants -1, 0, 1"),
    ] {
        let mut t = types();
        let ordering = t.recorded.intern(
            TypeShape::Adt {
                name: "core::cmp::Ordering".into(),
            },
            Some(SizeAlign { size: 1, align: 1 }),
        );
        t.recorded.record_aggregate(
            ordering,
            AggregateLayout {
                fields: vec![],
                variants: vec![vec![], vec![], vec![]],
            },
        );
        t.recorded.record_semantic_enum(ordering, discriminants);
        if marked {
            t.recorded.record_ordering(ordering);
        }
        let mut builder = BodyBuilder::new("invalid_compare", abi(2), t.recorded);
        let ret = local(&mut builder, ordering);
        let lhs = local(&mut builder, t.u8);
        let rhs = local(&mut builder, t.u8);
        builder.set_arg_count(2);
        assign(
            &mut builder,
            ret,
            Rvalue::BinaryOp(BinaryOp::Cmp, copy(lhs), copy(rhs)),
        );
        builder.finish_block(Terminator::Return, false);
        assert!(
            build_graph(&builder.finish())
                .unwrap_err()
                .to_string()
                .contains(reason)
        );
    }
}

#[test]
fn slice_pattern_subslice_updates_data_address_and_length() {
    let mut t = types();
    let usize_ty = t.recorded.intern(
        TypeShape::Int {
            width: IntegerWidth::Pointer,
            signed: false,
        },
        Some(SizeAlign { size: 8, align: 8 }),
    );
    let slice = t.recorded.intern(TypeShape::Slice { element: t.u8 }, None);
    let thin = t.recorded.intern(
        TypeShape::RawPointer {
            mutability: Mutability::Not,
            pointee: t.u8,
        },
        Some(SizeAlign { size: 8, align: 8 }),
    );
    let fat = t.recorded.intern(
        TypeShape::Reference {
            mutability: Mutability::Not,
            pointee: slice,
        },
        Some(SizeAlign { size: 16, align: 8 }),
    );
    t.recorded.record_aggregate(
        fat,
        AggregateLayout {
            fields: vec![
                FieldLayout {
                    ty: thin,
                    offset: 0,
                },
                FieldLayout {
                    ty: usize_ty,
                    offset: 8,
                },
            ],
            variants: vec![],
        },
    );
    let mut builder = BodyBuilder::new("slice_pattern", abi(0), t.recorded);
    let ret = local(&mut builder, usize_ty);
    let original = local(&mut builder, fat);
    let middle = local(&mut builder, fat);
    let length = local(&mut builder, usize_ty);
    let scaled = local(&mut builder, usize_ty);
    let last = local(&mut builder, usize_ty);
    builder.add_data_object(DataObject {
        symbol: "pattern_bytes".to_owned(),
        binding: DataBinding::Local,
        bytes: vec![1, 2, 3, 4, 5],
        align: 1,
        writable: false,
        thread_local: false,
        relocations: vec![],
    });
    let pointer = Operand::Constant(builder.add_constant(Constant {
        ty: thin,
        kind: ConstantKind::Address {
            symbol: "pattern_bytes".to_owned(),
            addend: 0,
        },
    }));
    let count = constant(&mut builder, usize_ty, 5);
    assign(
        &mut builder,
        original,
        Rvalue::Aggregate(AggregateKind::Tuple, vec![pointer, count]),
    );
    let subprojection = builder.add_projection(vec![
        ProjectionElem::Deref,
        ProjectionElem::Subslice {
            from: 1,
            to: 1,
            from_end: true,
            result_ty: slice,
        },
    ]);
    assign(
        &mut builder,
        middle,
        Rvalue::Ref(
            BorrowKind::Shared,
            Place {
                local: original,
                projection: subprojection,
            },
        ),
    );
    assign(
        &mut builder,
        length,
        Rvalue::UnaryOp(UnaryOp::PtrMetadata, copy(middle)),
    );
    let lastprojection = builder.add_projection(vec![
        ProjectionElem::Deref,
        ProjectionElem::ConstantIndex {
            offset: 1,
            min_length: 1,
            from_end: true,
        },
    ]);
    assign(
        &mut builder,
        last,
        Rvalue::Cast(
            CastKind::IntToInt,
            Operand::Copy(Place {
                local: middle,
                projection: lastprojection,
            }),
            usize_ty,
        ),
    );
    let hundred = constant(&mut builder, usize_ty, 100);
    assign(
        &mut builder,
        scaled,
        Rvalue::BinaryOp(BinaryOp::Mul, copy(length), hundred),
    );
    assign(
        &mut builder,
        ret,
        Rvalue::BinaryOp(BinaryOp::Add, copy(scaled), copy(last)),
    );
    builder.finish_block(Terminator::Return, false);
    let graph = build_verified(&builder.finish());
    assert_eq!(run(&graph, &[]), Outcome::Return(304));
}

#[test]
fn cast_abi_preserves_partial_lanes_padding_and_sign_extension() {
    for (size, lanes, pad_i32, extension, args, expected) in [
        (
            3,
            [
                Some(CastLane {
                    offset: 0,
                    bytes: 3,
                }),
                None,
            ],
            false,
            CastExtension::Sign,
            vec![0x00ff_ffff],
            Outcome::Return(u64::MAX),
        ),
        (
            8,
            [
                Some(CastLane {
                    offset: 0,
                    bytes: 8,
                }),
                None,
            ],
            true,
            CastExtension::None,
            vec![0, 0x1234_5678_9abc_def0],
            Outcome::Return(0x1234_5678_9abc_def0),
        ),
        (
            12,
            [
                Some(CastLane {
                    offset: 0,
                    bytes: 8,
                }),
                Some(CastLane {
                    offset: 8,
                    bytes: 8,
                }),
            ],
            false,
            CastExtension::None,
            vec![0x1234_5678_9abc_def0, 0xaaaa_bbbb_cccc_dddd],
            Outcome::ReturnPair(0x1234_5678_9abc_def0, 0xcccc_dddd),
        ),
    ] {
        let mut t = types();
        let aggregate = t.recorded.intern(
            TypeShape::Array {
                element: t.u8,
                length: size,
            },
            Some(SizeAlign { size, align: 1 }),
        );
        let cast = CastAbi {
            size,
            lanes,
            consecutive: false,
            pad_i32,
            extension,
        };
        let mut builder = BodyBuilder::new(
            "cast_identity",
            FunctionAbi {
                args: vec![PassMode::Cast(cast)],
                ret: PassMode::Cast(CastAbi {
                    pad_i32: false,
                    ..cast
                }),
                c_variadic: false,
            },
            t.recorded,
        );
        let ret = local(&mut builder, aggregate);
        let arg = local(&mut builder, aggregate);
        builder.set_arg_count(1);
        assign(&mut builder, ret, Rvalue::Use(copy(arg)));
        builder.finish_block(Terminator::Return, false);
        let graph = build_verified(&builder.finish());
        assert_eq!(
            run(&graph, &args),
            expected,
            "size={size},extension={extension:?}"
        );
    }
}

#[test]
fn wide_pointer_comparisons_use_data_and_metadata_instead_of_slot_addresses() {
    for op in [
        BinaryOp::Eq,
        BinaryOp::Ne,
        BinaryOp::Lt,
        BinaryOp::Le,
        BinaryOp::Gt,
        BinaryOp::Ge,
    ] {
        let mut t = types();
        let usize_ty = t.recorded.intern(
            TypeShape::Int {
                width: IntegerWidth::Pointer,
                signed: false,
            },
            Some(SizeAlign { size: 8, align: 8 }),
        );
        let slice = t.recorded.intern(TypeShape::Slice { element: t.u8 }, None);
        let thin = t.recorded.intern(
            TypeShape::RawPointer {
                mutability: Mutability::Not,
                pointee: t.u8,
            },
            Some(SizeAlign { size: 8, align: 8 }),
        );
        let fat = t.recorded.intern(
            TypeShape::RawPointer {
                mutability: Mutability::Not,
                pointee: slice,
            },
            Some(SizeAlign { size: 16, align: 8 }),
        );
        t.recorded.record_aggregate(
            fat,
            AggregateLayout {
                fields: vec![
                    FieldLayout {
                        ty: thin,
                        offset: 0,
                    },
                    FieldLayout {
                        ty: usize_ty,
                        offset: 8,
                    },
                ],
                variants: vec![],
            },
        );
        let mut builder = BodyBuilder::new("fat_compare", abi(4), t.recorded);
        let ret = local(&mut builder, t.bool);
        let data_left = local(&mut builder, usize_ty);
        let len_left = local(&mut builder, usize_ty);
        let data_right = local(&mut builder, usize_ty);
        let len_right = local(&mut builder, usize_ty);
        builder.set_arg_count(4);
        let left_ptr = local(&mut builder, thin);
        let right_ptr = local(&mut builder, thin);
        let left = local(&mut builder, fat);
        let right = local(&mut builder, fat);
        assign(
            &mut builder,
            left_ptr,
            Rvalue::Cast(CastKind::IntToInt, copy(data_left), thin),
        );
        assign(
            &mut builder,
            right_ptr,
            Rvalue::Cast(CastKind::IntToInt, copy(data_right), thin),
        );
        assign(
            &mut builder,
            left,
            Rvalue::Aggregate(AggregateKind::Tuple, vec![copy(left_ptr), copy(len_left)]),
        );
        assign(
            &mut builder,
            right,
            Rvalue::Aggregate(AggregateKind::Tuple, vec![copy(right_ptr), copy(len_right)]),
        );
        assign(
            &mut builder,
            ret,
            Rvalue::BinaryOp(op, copy(left), copy(right)),
        );
        builder.finish_block(Terminator::Return, false);
        let graph = build_verified(&builder.finish());
        for (a, b) in [
            ((100, 3), (100, 3)),
            ((100, 2), (100, 3)),
            ((100, 4), (100, 3)),
            ((99, 9), (100, 1)),
            ((101, 1), (100, 9)),
        ] {
            let expected = match op {
                BinaryOp::Eq => a == b,
                BinaryOp::Ne => a != b,
                BinaryOp::Lt => a < b,
                BinaryOp::Le => a <= b,
                BinaryOp::Gt => a > b,
                BinaryOp::Ge => a >= b,
                _ => unreachable!(),
            };
            assert_eq!(
                run(&graph, &[a.0, a.1, b.0, b.1]),
                Outcome::Return(u64::from(expected)),
                "{op:?},a={a:?},b={b:?}"
            );
        }
    }
}

#[test]
fn trait_object_vtable_relocation_loads_method_before_indirect_call() {
    let mut t = types();
    let dynamic = t.recorded.intern(
        TypeShape::Other {
            name: "dyn core::fmt::Debug".to_owned(),
        },
        None,
    );
    let data_pointer = t.recorded.intern(
        TypeShape::RawPointer {
            mutability: Mutability::Not,
            pointee: t.i64,
        },
        Some(SizeAlign { size: 8, align: 8 }),
    );
    let unit_pointer = t.recorded.intern(
        TypeShape::RawPointer {
            mutability: Mutability::Not,
            pointee: t.unit,
        },
        Some(SizeAlign { size: 8, align: 8 }),
    );
    let vtable = t.recorded.intern(
        TypeShape::Array {
            element: unit_pointer,
            length: 4,
        },
        Some(SizeAlign { size: 32, align: 8 }),
    );
    let metadata_pointer = t.recorded.intern(
        TypeShape::RawPointer {
            mutability: Mutability::Not,
            pointee: vtable,
        },
        Some(SizeAlign { size: 8, align: 8 }),
    );
    let object = t.recorded.intern(
        TypeShape::Reference {
            mutability: Mutability::Not,
            pointee: dynamic,
        },
        Some(SizeAlign { size: 16, align: 8 }),
    );
    t.recorded.record_aggregate(
        object,
        AggregateLayout {
            fields: vec![
                FieldLayout {
                    ty: data_pointer,
                    offset: 0,
                },
                FieldLayout {
                    ty: metadata_pointer,
                    offset: 8,
                },
            ],
            variants: vec![],
        },
    );
    let mut builder = BodyBuilder::new("dynamic_call", abi(0), t.recorded);
    let ret = local(&mut builder, t.i64);
    let data = local(&mut builder, t.i64);
    let data_address = local(&mut builder, data_pointer);
    let trait_object = local(&mut builder, object);
    let table_address = local(&mut builder, metadata_pointer);
    let method = local(&mut builder, unit_pointer);
    let raw_data = local(&mut builder, data_pointer);
    let before = constant(&mut builder, t.i64, 7);
    assign(&mut builder, data, Rvalue::Use(before));
    assign(
        &mut builder,
        data_address,
        Rvalue::RawPointer(Mutability::Not, Place::local(data)),
    );
    builder.add_data_object(DataObject {
        symbol: "debug_vtable".to_owned(),
        binding: DataBinding::Local,
        bytes: vec![0; 32],
        align: 8,
        writable: false,
        thread_local: false,
        relocations: vec![DataRelocation {
            offset: 24,
            target: "debug_method".to_owned(),
            addend: 0,
        }],
    });
    let metadata = Operand::Constant(builder.add_constant(Constant {
        ty: metadata_pointer,
        kind: ConstantKind::Address {
            symbol: "debug_vtable".to_owned(),
            addend: 0,
        },
    }));
    assign(
        &mut builder,
        trait_object,
        Rvalue::Aggregate(AggregateKind::Tuple, vec![copy(data_address), metadata]),
    );
    let after = constant(&mut builder, t.i64, 42);
    assign(&mut builder, data, Rvalue::Use(after));
    assign(
        &mut builder,
        table_address,
        Rvalue::UnaryOp(UnaryOp::PtrMetadata, copy(trait_object)),
    );
    let slot = builder.add_projection(vec![
        ProjectionElem::Deref,
        ProjectionElem::ConstantIndex {
            offset: 3,
            min_length: 4,
            from_end: false,
        },
    ]);
    assign(
        &mut builder,
        method,
        Rvalue::Use(Operand::Copy(Place {
            local: table_address,
            projection: slot,
        })),
    );
    let field = builder.add_projection(vec![ProjectionElem::Field(FieldIndex(0), data_pointer)]);
    assign(
        &mut builder,
        raw_data,
        Rvalue::Use(Operand::Copy(Place {
            local: trait_object,
            projection: field,
        })),
    );
    let deref = builder.add_projection(vec![ProjectionElem::Deref]);
    builder.finish_block(
        Terminator::CallIndirect {
            function: copy(method),
            abi: abi(1),
            args: vec![Operand::Copy(Place {
                local: raw_data,
                projection: deref,
            })],
            destination: Place::local(ret),
            target: Some(BlockId(1)),
            unwind: UnwindAction::Unreachable,
        },
        false,
    );
    builder.finish_block(Terminator::Return, false);
    let graph = build_verified(&builder.finish());
    let mut called = false;
    let mut calls = |symbol: &str, args: &[u64]| {
        called = true;
        assert_eq!(symbol, "debug_method");
        assert_eq!(args, &[42]);
        Ok(args[0] + 1)
    };
    assert_eq!(
        Interpreter::new(&layout_graph(&graph))
            .run(&[], &mut calls)
            .unwrap(),
        Outcome::Return(43)
    );
    assert!(called);
}

#[test]
fn zero_sized_aggregate_copies_and_ignored_calls_preserve_effects() {
    let mut t = types();
    let closure = t.recorded.intern(
        TypeShape::Adt {
            name: "zero_capture_closure".to_owned(),
        },
        Some(SizeAlign { size: 0, align: 1 }),
    );
    t.recorded
        .record_aggregate(closure, AggregateLayout::default());
    let mut builder = BodyBuilder::new("zst", abi(0), t.recorded);
    let ret = local(&mut builder, t.i64);
    let first = local(&mut builder, closure);
    let second = local(&mut builder, closure);
    assign(
        &mut builder,
        first,
        Rvalue::Aggregate(AggregateKind::Closure(closure), vec![]),
    );
    assign(&mut builder, second, Rvalue::Use(copy(first)));
    let callee = Operand::Constant(builder.add_constant(Constant {
        ty: t.unit,
        kind: ConstantKind::Function(Callee::Function {
            symbol: "consume_zst".to_owned(),
            abi: FunctionAbi {
                args: vec![PassMode::Ignore],
                ret: PassMode::Ignore,
                c_variadic: false,
            },
        }),
    }));
    builder.finish_block(
        Terminator::Call {
            callee,
            args: vec![copy(second)],
            destination: Place::local(second),
            target: Some(BlockId(1)),
            unwind: UnwindAction::Unreachable,
        },
        false,
    );
    let result = constant(&mut builder, t.i64, 42);
    assign(&mut builder, ret, Rvalue::Use(result));
    builder.finish_block(Terminator::Return, false);
    let graph = build_verified(&builder.finish());
    let mut called = false;
    let mut calls = |symbol: &str, args: &[u64]| {
        called = true;
        assert_eq!(symbol, "consume_zst");
        assert!(args.is_empty());
        Ok(0)
    };
    assert_eq!(
        Interpreter::new(&layout_graph(&graph))
            .run(&[], &mut calls)
            .unwrap(),
        Outcome::Return(42)
    );
    assert!(called);
}

/// Exercise the unique value pipeline before interpreting its physical ABI.
fn layout_graph(graph: &kfrc_son::FunctionGraph) -> kfrc_son::FunctionGraph {
    let mut graph = graph.clone();
    kfrc_son::verify_graph(&mut graph).unwrap();
    kfrc_son::lower_layout(&mut graph).unwrap();
    kfrc_son::verify_graph(&mut graph).unwrap();
    graph
}
