//! The builder preserves Rust values without consulting physical layouts.

use kfrc_body::*;
use kfrc_son::{
    FunctionGraph, GraphLevel, InterpretError, Interpreter, Opcode, SemanticOutcome, Value,
    build_graph, verify_graph,
};

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

fn scalar(builder: &mut BodyBuilder, ty: TyId, bits: u128) -> Operand {
    Operand::Constant(builder.add_constant(Constant {
        ty,
        kind: ConstantKind::Scalar(bits),
    }))
}

fn field(builder: &mut BodyBuilder, local: LocalId, index: u32, ty: TyId) -> Place {
    Place {
        local,
        projection: builder.add_projection(vec![ProjectionElem::Field(FieldIndex(index), ty)]),
    }
}

fn assign(builder: &mut BodyBuilder, place: Place, value: Rvalue) {
    builder.push_statement(StatementKind::Assign(place, value));
}

fn types() -> (RecordedTypes, TyId, TyId, TyId) {
    let mut types = RecordedTypes::new();
    let word = types.intern(
        TypeShape::Int {
            width: IntegerWidth::W64,
            signed: false,
        },
        None,
    );
    let boolean = types.intern(TypeShape::Bool, None);
    let pair = types.intern(TypeShape::Tuple(vec![word, word]), None);
    (types, word, boolean, pair)
}

fn builder(symbol: &str, types: RecordedTypes, args: Vec<PassMode>, ret: PassMode) -> BodyBuilder {
    BodyBuilder::new(
        symbol,
        FunctionAbi {
            args,
            ret,
            c_variadic: false,
        },
        types,
    )
}

fn graph(body: &Body) -> FunctionGraph {
    let mut graph = build_graph(body).unwrap();
    assert_eq!(graph.level, GraphLevel::Specialized);
    assert!(graph.stack_slots.is_empty());
    assert!(!graph.ids().any(|node| matches!(
        graph.op(node),
        Opcode::StackSlot | Opcode::Load | Opcode::Store | Opcode::PtrOffset
    )));
    verify_graph(&mut graph).unwrap();
    graph
}

fn run(graph: &FunctionGraph, args: &[Value]) -> Result<SemanticOutcome, InterpretError> {
    Interpreter::new(graph).run_values(args, &mut |symbol: &str, _: &[Value]| {
        Err(InterpretError::UnknownCallee(symbol.into()))
    })
}

#[test]
fn partial_initialization_and_move_keep_snapshot_fields_independent() {
    let (mut types, word, _, _) = types();
    let record = types.intern(
        TypeShape::Adt {
            name: "RecordWithoutLayout".into(),
        },
        None,
    );
    types.record_semantic_aggregate(
        record,
        SemanticAggregate {
            fields: vec![word, word],
            variants: vec![],
        },
    );
    let mut b = builder(
        "snapshot",
        types,
        vec![PassMode::Direct, PassMode::Direct],
        PassMode::Direct,
    );
    let ret = local(&mut b, word);
    let first = local(&mut b, word);
    let second = local(&mut b, word);
    b.set_arg_count(2);
    let record_local = local(&mut b, record);
    let snapshot = local(&mut b, record);
    let a = field(&mut b, record_local, 0, word);
    assign(&mut b, a, Rvalue::Use(copy(first)));
    assign(
        &mut b,
        Place::local(snapshot),
        Rvalue::Use(Operand::Move(Place::local(record_local))),
    );
    assign(&mut b, a, Rvalue::Use(copy(second)));
    let old = field(&mut b, snapshot, 0, word);
    assign(&mut b, Place::local(ret), Rvalue::Use(Operand::Copy(old)));
    b.finish_block(Terminator::Return, false);
    let graph = graph(&b.finish());
    assert!(
        graph
            .ids()
            .any(|node| graph.op(node) == Opcode::InsertField)
    );
    assert_eq!(
        run(&graph, &[Value::Scalar(17), Value::Scalar(99)]).unwrap(),
        SemanticOutcome::Return(Value::Scalar(17))
    );
}

#[test]
fn loop_phi_carries_aggregate_values_and_updates_simultaneously() {
    let (types, word, boolean, pair) = types();
    let mut b = builder("fibonacci", types, vec![PassMode::Direct], PassMode::Direct);
    let ret = local(&mut b, word);
    let bound = local(&mut b, word);
    b.set_arg_count(1);
    let state = local(&mut b, pair);
    let count = local(&mut b, word);
    let next = local(&mut b, word);
    let condition = local(&mut b, boolean);
    let zero = scalar(&mut b, word, 0);
    let one = scalar(&mut b, word, 1);
    assign(
        &mut b,
        Place::local(state),
        Rvalue::Aggregate(AggregateKind::Tuple, vec![zero, one]),
    );
    assign(&mut b, Place::local(count), Rvalue::Use(zero));
    b.finish_block(Terminator::Goto(BlockId(1)), false);
    assign(
        &mut b,
        Place::local(condition),
        Rvalue::BinaryOp(BinaryOp::Lt, copy(count), copy(bound)),
    );
    b.finish_block(
        Terminator::SwitchInt {
            discriminant: copy(condition),
            targets: SwitchTargets {
                cases: vec![(1, BlockId(2))],
                otherwise: BlockId(3),
            },
        },
        false,
    );
    let a = field(&mut b, state, 0, word);
    let z = field(&mut b, state, 1, word);
    assign(
        &mut b,
        Place::local(next),
        Rvalue::BinaryOp(BinaryOp::Add, Operand::Copy(a), Operand::Copy(z)),
    );
    assign(
        &mut b,
        Place::local(state),
        Rvalue::Aggregate(AggregateKind::Tuple, vec![Operand::Copy(z), copy(next)]),
    );
    assign(
        &mut b,
        Place::local(count),
        Rvalue::BinaryOp(BinaryOp::Add, copy(count), one),
    );
    b.finish_block(Terminator::Goto(BlockId(1)), false);
    assign(&mut b, Place::local(ret), Rvalue::Use(Operand::Copy(a)));
    b.finish_block(Terminator::Return, false);
    let graph = graph(&b.finish());
    assert!(
        graph
            .ids()
            .any(|node| graph.op(node) == Opcode::Phi && graph.rust_type(node) == Some(pair))
    );
    for (n, expected) in [(0, 0), (1, 1), (2, 1), (10, 55), (20, 6765)] {
        assert_eq!(
            run(&graph, &[Value::Scalar(n)]).unwrap(),
            SemanticOutcome::Return(Value::Scalar(expected))
        );
    }
}

#[test]
fn calls_pass_one_aggregate_value_without_snapshots_or_abi_lanes() {
    let (mut types, word, _, pair) = types();
    let unit = types.intern(TypeShape::Unit, None);
    let mut b = builder(
        "aggregate_call",
        types,
        vec![PassMode::Pair],
        PassMode::Direct,
    );
    let ret = local(&mut b, word);
    let arg = local(&mut b, pair);
    b.set_arg_count(1);
    let output = local(&mut b, pair);
    let function = Operand::Constant(b.add_constant(Constant {
        ty: unit,
        kind: ConstantKind::Function(Callee::Function {
            symbol: "swap".into(),
            abi: FunctionAbi {
                args: vec![PassMode::Pair],
                ret: PassMode::Pair,
                c_variadic: false,
            },
        }),
    }));
    b.finish_block(
        Terminator::Call {
            callee: function,
            args: vec![copy(arg)],
            destination: Place::local(output),
            target: Some(BlockId(1)),
            unwind: UnwindAction::Unreachable,
        },
        false,
    );
    let last = field(&mut b, output, 1, word);
    assign(&mut b, Place::local(ret), Rvalue::Use(Operand::Copy(last)));
    b.finish_block(Terminator::Return, false);
    let graph = graph(&b.finish());
    let result = Interpreter::new(&graph)
        .run_values(
            &[Value::Aggregate(vec![Value::Scalar(41), Value::Scalar(73)])],
            &mut |symbol: &str, args: &[Value]| {
                assert_eq!(symbol, "swap");
                let [Value::Aggregate(fields)] = args else {
                    panic!("semantic call must retain its one aggregate parameter")
                };
                Ok(Value::Aggregate(vec![fields[1].clone(), fields[0].clone()]))
            },
        )
        .unwrap();
    assert_eq!(result, SemanticOutcome::Return(Value::Scalar(41)));
}

#[test]
fn enum_fields_and_user_discriminants_do_not_decode_a_physical_tag() {
    let (mut types, word, _, _) = types();
    let enumeration = types.intern(
        TypeShape::Adt {
            name: "SemanticEnum".into(),
        },
        None,
    );
    types.record_semantic_aggregate(
        enumeration,
        SemanticAggregate {
            fields: vec![],
            variants: vec![vec![], vec![word]],
        },
    );
    types.record_semantic_enum(enumeration, vec![7, 19]);
    let mut b = builder("variant", types, vec![PassMode::Direct], PassMode::Direct);
    let ret = local(&mut b, word);
    let arg = local(&mut b, word);
    b.set_arg_count(1);
    let value = local(&mut b, enumeration);
    let discriminant = local(&mut b, word);
    assign(
        &mut b,
        Place::local(value),
        Rvalue::Aggregate(
            AggregateKind::Adt {
                ty: enumeration,
                variant: VariantIndex(1),
            },
            vec![copy(arg)],
        ),
    );
    assign(
        &mut b,
        Place::local(discriminant),
        Rvalue::Discriminant(Place::local(value)),
    );
    let payload = Place {
        local: value,
        projection: b.add_projection(vec![
            ProjectionElem::Downcast(VariantIndex(1)),
            ProjectionElem::Field(FieldIndex(0), word),
        ]),
    };
    assign(
        &mut b,
        Place::local(ret),
        Rvalue::BinaryOp(BinaryOp::Add, copy(discriminant), Operand::Move(payload)),
    );
    b.finish_block(Terminator::Return, false);
    let graph = graph(&b.finish());
    assert_eq!(
        run(&graph, &[Value::Scalar(23)]).unwrap(),
        SemanticOutcome::Return(Value::Scalar(42))
    );
}

#[test]
fn borrowed_aggregate_loads_are_snapshots_and_dynamic_indices_address_elements() {
    let (mut types, word, _, pair) = types();
    // Addressable memory requires producer layout answers at interpretation /
    // lowering. Pure-value tests above intentionally omit those answers.
    types.intern(
        TypeShape::Int {
            width: IntegerWidth::W64,
            signed: false,
        },
        Some(SizeAlign { size: 8, align: 8 }),
    );
    types.intern(
        TypeShape::Tuple(vec![word, word]),
        Some(SizeAlign { size: 16, align: 8 }),
    );
    types.record_aggregate(
        pair,
        AggregateLayout {
            fields: vec![
                FieldLayout {
                    ty: word,
                    offset: 0,
                },
                FieldLayout {
                    ty: word,
                    offset: 8,
                },
            ],
            variants: vec![],
        },
    );
    let pointer = types.intern(
        TypeShape::RawPointer {
            mutability: Mutability::Mut,
            pointee: pair,
        },
        Some(SizeAlign { size: 8, align: 8 }),
    );
    let array = types.intern(
        TypeShape::Array {
            element: word,
            length: 3,
        },
        Some(SizeAlign { size: 24, align: 8 }),
    );
    let mut b = builder(
        "borrow_snapshot",
        types,
        vec![PassMode::Direct],
        PassMode::Direct,
    );
    let ret = local(&mut b, word);
    let index = local(&mut b, word);
    b.set_arg_count(1);
    let source = local(&mut b, pair);
    let borrowed = local(&mut b, pointer);
    let snapshot = local(&mut b, pair);
    let items = local(&mut b, array);
    let eleven = scalar(&mut b, word, 11);
    let twenty = scalar(&mut b, word, 20);
    let ninety = scalar(&mut b, word, 90);
    assign(
        &mut b,
        Place::local(source),
        Rvalue::Aggregate(AggregateKind::Tuple, vec![eleven, twenty]),
    );
    assign(
        &mut b,
        Place::local(borrowed),
        Rvalue::RawPointer(Mutability::Mut, Place::local(source)),
    );
    let deref = Place {
        local: borrowed,
        projection: b.add_projection(vec![ProjectionElem::Deref]),
    };
    assign(
        &mut b,
        Place::local(snapshot),
        Rvalue::Use(Operand::Copy(deref)),
    );
    let first = Place {
        local: borrowed,
        projection: b.add_projection(vec![
            ProjectionElem::Deref,
            ProjectionElem::Field(FieldIndex(0), word),
        ]),
    };
    assign(&mut b, first, Rvalue::Use(ninety));
    assign(
        &mut b,
        Place::local(items),
        Rvalue::Aggregate(AggregateKind::Array(word), vec![eleven, twenty, ninety]),
    );
    let indexed = Place {
        local: items,
        projection: b.add_projection(vec![ProjectionElem::Index(index)]),
    };
    let old = field(&mut b, snapshot, 0, word);
    assign(
        &mut b,
        Place::local(ret),
        Rvalue::BinaryOp(BinaryOp::Add, Operand::Copy(old), Operand::Copy(indexed)),
    );
    b.finish_block(Terminator::Return, false);
    let mut graph = build_graph(&b.finish()).unwrap();
    verify_graph(&mut graph).unwrap();
    assert!(graph.stack_slots.is_empty());
    assert_eq!(
        graph
            .ids()
            .filter(|node| graph.op(*node) == Opcode::LocalSlot)
            .count(),
        2
    );
    assert!(graph.ids().any(|node| graph.op(node) == Opcode::LoadValue));
    for (index, expected) in [(0, 22), (1, 31), (2, 101)] {
        assert_eq!(
            run(&graph, &[Value::Scalar(index)]).unwrap(),
            SemanticOutcome::Return(Value::Scalar(expected))
        );
    }
}

#[test]
fn nested_array_field_updates_preserve_unmodified_value_snapshots() {
    let (mut types, word, _, pair) = types();
    let array = types.intern(
        TypeShape::Array {
            element: pair,
            length: 2,
        },
        None,
    );
    let mut b = builder("nested_array", types, vec![], PassMode::Direct);
    let ret = local(&mut b, word);
    let item = local(&mut b, pair);
    let items = local(&mut b, array);
    let snapshot = local(&mut b, array);
    let first = scalar(&mut b, word, 11);
    let second = scalar(&mut b, word, 22);
    let updated = scalar(&mut b, word, 99);
    assign(
        &mut b,
        Place::local(item),
        Rvalue::Aggregate(AggregateKind::Tuple, vec![first, second]),
    );
    assign(&mut b, Place::local(items), Rvalue::Repeat(copy(item), 2));
    assign(&mut b, Place::local(snapshot), Rvalue::Use(copy(items)));
    let nested_projection = b.add_projection(vec![
        ProjectionElem::ConstantIndex {
            offset: 1,
            min_length: 2,
            from_end: true,
        },
        ProjectionElem::Field(FieldIndex(1), word),
    ]);
    assign(
        &mut b,
        Place {
            local: items,
            projection: nested_projection,
        },
        Rvalue::Use(updated),
    );
    assign(
        &mut b,
        Place::local(ret),
        Rvalue::BinaryOp(
            BinaryOp::Add,
            Operand::Copy(Place {
                local: items,
                projection: nested_projection,
            }),
            Operand::Copy(Place {
                local: snapshot,
                projection: nested_projection,
            }),
        ),
    );
    b.finish_block(Terminator::Return, false);
    let graph = graph(&b.finish());
    assert_eq!(
        run(&graph, &[]).unwrap(),
        SemanticOutcome::Return(Value::Scalar(121))
    );
}

#[test]
fn wide_pointer_metadata_is_a_value_projection_without_loading_memory() {
    let (mut types, word, _, _) = types();
    let slice = types.intern(TypeShape::Slice { element: word }, None);
    let thin = types.intern(
        TypeShape::RawPointer {
            mutability: Mutability::Not,
            pointee: word,
        },
        None,
    );
    let wide = types.intern(
        TypeShape::RawPointer {
            mutability: Mutability::Not,
            pointee: slice,
        },
        None,
    );
    types.record_semantic_aggregate(
        wide,
        SemanticAggregate {
            fields: vec![thin, word],
            variants: vec![],
        },
    );
    let mut b = builder("metadata", types, vec![PassMode::Pair], PassMode::Direct);
    let ret = local(&mut b, word);
    let pointer = local(&mut b, wide);
    b.set_arg_count(1);
    assign(
        &mut b,
        Place::local(ret),
        Rvalue::UnaryOp(UnaryOp::PtrMetadata, copy(pointer)),
    );
    b.finish_block(Terminator::Return, false);
    let graph = graph(&b.finish());
    assert_eq!(
        run(
            &graph,
            &[Value::WidePointer {
                data: Box::new(Value::Scalar(0x1234)),
                metadata: Box::new(Value::Scalar(37))
            }]
        )
        .unwrap(),
        SemanticOutcome::Return(Value::Scalar(37))
    );
}

#[test]
fn exposed_pointer_arithmetic_uses_integer_casts_and_preserves_wrapping() {
    let (mut types, word, _, _) = types();
    let pointer = types.intern(
        TypeShape::RawPointer {
            mutability: Mutability::Not,
            pointee: word,
        },
        None,
    );
    let mut b = builder(
        "exposed_address",
        types,
        vec![PassMode::Direct],
        PassMode::Direct,
    );
    let ret = local(&mut b, pointer);
    let input = local(&mut b, pointer);
    let bits = local(&mut b, word);
    b.set_arg_count(1);
    assign(
        &mut b,
        Place::local(bits),
        Rvalue::Cast(CastKind::IntToInt, copy(input), word),
    );
    let offset = scalar(&mut b, word, 16);
    assign(
        &mut b,
        Place::local(bits),
        Rvalue::BinaryOp(BinaryOp::Add, copy(bits), offset),
    );
    assign(
        &mut b,
        Place::local(ret),
        Rvalue::Cast(CastKind::IntToInt, copy(bits), pointer),
    );
    b.finish_block(Terminator::Return, false);
    let graph = graph(&b.finish());
    assert!(
        !graph
            .ids()
            .any(|node| graph.op(node) == Opcode::PointerCast)
    );
    for (address, expected) in [(0x1234, 0x1244), (u64::MAX - 7, 8)] {
        assert_eq!(
            run(&graph, &[Value::Scalar(address)]).unwrap(),
            SemanticOutcome::Return(Value::Scalar(expected))
        );
    }
}

#[test]
fn malformed_pointer_cast_from_integer_is_rejected_at_build_boundary() {
    let (mut types, word, _, _) = types();
    let pointer = types.intern(
        TypeShape::RawPointer {
            mutability: Mutability::Not,
            pointee: word,
        },
        None,
    );
    let mut b = builder(
        "invalid_pointer_cast",
        types,
        vec![PassMode::Direct],
        PassMode::Direct,
    );
    let ret = local(&mut b, pointer);
    let input = local(&mut b, word);
    b.set_arg_count(1);
    assign(
        &mut b,
        Place::local(ret),
        Rvalue::Cast(CastKind::PtrToPtr, copy(input), pointer),
    );
    b.finish_block(Terminator::Return, false);
    assert!(
        build_graph(&b.finish())
            .unwrap_err()
            .to_string()
            .contains("exposed address")
    );
}

#[test]
fn pointer_offset_preserves_element_semantics_for_negative_wrapping_and_zst() {
    for size in [0, 8] {
        let (mut types, word, _, _) = types();
        types.intern(
            TypeShape::Int {
                width: IntegerWidth::W64,
                signed: false,
            },
            Some(SizeAlign { size: 8, align: 8 }),
        );
        let element = if size == 0 {
            types.intern(TypeShape::Unit, Some(SizeAlign { size: 0, align: 1 }))
        } else {
            word
        };
        let offset = types.intern(
            TypeShape::Int {
                width: IntegerWidth::Pointer,
                signed: true,
            },
            Some(SizeAlign { size: 8, align: 8 }),
        );
        let pointer = types.intern(
            TypeShape::RawPointer {
                mutability: Mutability::Not,
                pointee: element,
            },
            Some(SizeAlign { size: 8, align: 8 }),
        );
        let mut b = builder(
            "pointer_offset",
            types,
            vec![PassMode::Direct, PassMode::Direct],
            PassMode::Direct,
        );
        let ret = local(&mut b, pointer);
        let input = local(&mut b, pointer);
        let index = local(&mut b, offset);
        b.set_arg_count(2);
        assign(
            &mut b,
            Place::local(ret),
            Rvalue::BinaryOp(BinaryOp::Offset, copy(input), copy(index)),
        );
        b.finish_block(Terminator::Return, false);
        let graph = graph(&b.finish());
        assert!(graph.ids().any(|node| graph.op(node) == Opcode::IndexAddr));
        for (address, index) in [(0x1000u64, -2i64), (u64::MAX - 3, 1), (0, -1)] {
            assert_eq!(
                run(
                    &graph,
                    &[Value::Scalar(address), Value::Scalar(index as u64)]
                )
                .unwrap(),
                SemanticOutcome::Return(Value::Scalar(
                    address.wrapping_add((index as u64).wrapping_mul(size))
                ))
            );
        }
    }
}

#[test]
fn repeat_construction_and_field_folding_do_not_scale_with_array_length() {
    let mut baseline = None;
    for length in [1, 4096, u64::from(u32::MAX) + 1] {
        let (mut types, word, _, _) = types();
        let array = types.intern(
            TypeShape::Array {
                element: word,
                length,
            },
            None,
        );
        let mut b = builder(
            "compact_repeat",
            types,
            vec![PassMode::Direct],
            PassMode::Direct,
        );
        let ret = local(&mut b, word);
        let input = local(&mut b, word);
        let repeated = local(&mut b, array);
        b.set_arg_count(1);
        assign(
            &mut b,
            Place::local(repeated),
            Rvalue::Repeat(copy(input), length),
        );
        let last = b.add_projection(vec![ProjectionElem::ConstantIndex {
            offset: length - 1,
            min_length: length,
            from_end: false,
        }]);
        assign(
            &mut b,
            Place::local(ret),
            Rvalue::Use(Operand::Copy(Place {
                local: repeated,
                projection: last,
            })),
        );
        b.finish_block(Terminator::Return, false);
        let mut graph = graph(&b.finish());
        let repeat = graph
            .ids()
            .find(|&n| graph.op(n) == Opcode::Repeat)
            .unwrap();
        assert_eq!(graph.inputs(repeat).len(), 1);
        assert_eq!(
            graph.ids().count(),
            *baseline.get_or_insert_with(|| graph.ids().count())
        );
        assert_eq!(
            run(&graph, &[Value::Scalar(93)]).unwrap(),
            SemanticOutcome::Return(Value::Scalar(93))
        );
        kfrc_son::optimize_graph(&mut graph);
        verify_graph(&mut graph).unwrap();
        assert!(
            !graph
                .ids()
                .any(|n| matches!(graph.op(n), Opcode::Field | Opcode::Repeat))
        );
        assert_eq!(
            run(&graph, &[Value::Scalar(93)]).unwrap(),
            SemanticOutcome::Return(Value::Scalar(93))
        );
    }
}

#[test]
fn structurally_empty_array_constants_keep_repetition_compact() {
    let mut baseline = None;
    for length in [1, 4096, u64::from(u32::MAX) + 1] {
        let (mut types, _, _, _) = types();
        let unit = types.intern(TypeShape::Unit, None);
        let array = types.intern(
            TypeShape::Array {
                element: unit,
                length,
            },
            None,
        );
        let mut b = builder("empty_repeat", types, vec![], PassMode::Direct);
        let ret = local(&mut b, array);
        let empty = b.add_constant(Constant {
            ty: array,
            kind: ConstantKind::ZeroSized,
        });
        assign(
            &mut b,
            Place::local(ret),
            Rvalue::Use(Operand::Constant(empty)),
        );
        b.finish_block(Terminator::Return, false);
        let graph = graph(&b.finish());
        assert_eq!(
            graph.ids().count(),
            *baseline.get_or_insert_with(|| graph.ids().count())
        );
        assert!(
            graph
                .ids()
                .any(|n| graph.op(n) == Opcode::Repeat && graph.inputs(n).len() == 1)
        );
        let SemanticOutcome::Return(Value::Array {
            element,
            length: actual,
            overrides,
        }) = run(&graph, &[]).unwrap()
        else {
            panic!("expected compact repeated array");
        };
        assert_eq!(actual, length);
        assert_eq!(*element, Value::Aggregate(vec![]));
        assert!(overrides.is_empty());
    }
}

#[test]
fn zero_length_array_of_nonempty_elements_needs_no_element_value() {
    for construction in 0..3 {
        let (mut types, word, _, _) = types();
        let array = types.intern(
            TypeShape::Array {
                element: word,
                length: 0,
            },
            None,
        );
        let mut b = builder("zero_length", types, vec![], PassMode::Direct);
        let ret = local(&mut b, array);
        let empty = b.add_constant(Constant {
            ty: array,
            kind: ConstantKind::ZeroSized,
        });
        let value = match construction {
            0 => Rvalue::Use(Operand::Constant(empty)),
            1 => Rvalue::Aggregate(AggregateKind::Array(word), vec![]),
            _ => {
                let uninitialized = local(&mut b, word);
                Rvalue::Repeat(copy(uninitialized), 0)
            }
        };
        assign(&mut b, Place::local(ret), value);
        b.finish_block(Terminator::Return, false);
        let graph = graph(&b.finish());
        let repeat = graph
            .ids()
            .find(|&n| graph.op(n) == Opcode::Repeat)
            .unwrap();
        assert_eq!(graph.op(graph.inputs(repeat)[0]), Opcode::Undefined);
        let SemanticOutcome::Return(Value::Array {
            length, overrides, ..
        }) = run(&graph, &[]).unwrap()
        else {
            panic!("expected compact empty array");
        };
        assert_eq!(length, 0);
        assert!(overrides.is_empty());
    }
}

#[test]
fn zero_sized_enum_uses_the_producers_unique_inhabited_variant() {
    for call_result in [false, true] {
        let (mut types, _, _, _) = types();
        let unit = types.intern(TypeShape::Unit, None);
        let impossible = types.intern(
            TypeShape::Adt {
                name: "Infallible".into(),
            },
            None,
        );
        types.record_semantic_aggregate(
            impossible,
            SemanticAggregate {
                fields: vec![],
                variants: vec![],
            },
        );
        types.record_semantic_enum(impossible, vec![]);
        types.record_semantic_inhabited_variants(impossible, vec![]);
        let option = types.intern(
            TypeShape::Adt {
                name: "OptionInfallibleReversed".into(),
            },
            None,
        );
        // Deliberately put the unique inhabited case at index one. Selecting
        // zero or counting source variants would both be incorrect.
        types.record_semantic_aggregate(
            option,
            SemanticAggregate {
                fields: vec![],
                variants: vec![vec![impossible], vec![unit]],
            },
        );
        types.record_semantic_enum(option, vec![7, 19]);
        types.record_semantic_inhabited_variants(option, vec![1]);
        let mut b = builder("inhabited_zst", types, vec![], PassMode::Direct);
        let ret = local(&mut b, option);
        if call_result {
            let function = Operand::Constant(b.add_constant(Constant {
                ty: unit,
                kind: ConstantKind::Function(Callee::Function {
                    symbol: "empty_option".into(),
                    abi: FunctionAbi {
                        args: vec![],
                        ret: PassMode::Ignore,
                        c_variadic: false,
                    },
                }),
            }));
            b.finish_block(
                Terminator::Call {
                    callee: function,
                    args: vec![],
                    destination: Place::local(ret),
                    target: Some(BlockId(1)),
                    unwind: UnwindAction::Unreachable,
                },
                false,
            );
        } else {
            let constant = b.add_constant(Constant {
                ty: option,
                kind: ConstantKind::ZeroSized,
            });
            assign(
                &mut b,
                Place::local(ret),
                Rvalue::Use(Operand::Constant(constant)),
            );
        }
        b.finish_block(Terminator::Return, false);
        let graph = graph(&b.finish());
        let result = Interpreter::new(&graph).run_values(&[], &mut |name: &str, _: &[Value]| {
            assert_eq!(name, "empty_option");
            Ok(Value::Variant {
                variant: 1,
                fields: vec![Value::Aggregate(vec![])],
            })
        });
        assert_eq!(
            result.unwrap(),
            SemanticOutcome::Return(Value::Variant {
                variant: 1,
                fields: vec![Value::Aggregate(vec![])]
            })
        );
        if call_result {
            let call = graph
                .ids()
                .find(|&node| graph.op(node) == Opcode::Call)
                .unwrap();
            assert!(graph.find_projection(call, 2).is_some());
        } else {
            assert!(
                graph
                    .ids()
                    .any(|node| graph.op(node) == Opcode::MakeVariant && graph.payload(node) == 1)
            );
        }
    }
}

#[test]
fn ambiguous_or_uninhabited_zero_sized_enums_are_not_given_a_guessed_variant() {
    for inhabited in [None, Some(vec![]), Some(vec![0, 1])] {
        let (mut types, _, _, _) = types();
        let enumeration = types.intern(
            TypeShape::Adt {
                name: "NoUniqueVariant".into(),
            },
            None,
        );
        types.record_semantic_aggregate(
            enumeration,
            SemanticAggregate {
                fields: vec![],
                variants: vec![vec![], vec![]],
            },
        );
        types.record_semantic_enum(enumeration, vec![7, 19]);
        if let Some(inhabited) = inhabited {
            types.record_semantic_inhabited_variants(enumeration, inhabited);
        }
        let mut b = builder("no_guessed_variant", types, vec![], PassMode::Direct);
        let ret = local(&mut b, enumeration);
        let constant = b.add_constant(Constant {
            ty: enumeration,
            kind: ConstantKind::ZeroSized,
        });
        assign(
            &mut b,
            Place::local(ret),
            Rvalue::Use(Operand::Constant(constant)),
        );
        b.finish_block(Terminator::Return, false);
        assert!(
            build_graph(&b.finish())
                .unwrap_err()
                .to_string()
                .contains("unique inhabited variant")
        );
    }
}

#[test]
fn ignored_abi_return_preserves_and_observes_its_nonunit_source_value() {
    for initialized in [false, true] {
        let (mut types, _, _, _) = types();
        let unit = types.intern(TypeShape::Unit, None);
        // An uninitialized product of units has no poison leaf. An enum
        // still has an unknown logical variant, even when its one inhabited
        // case has zero-sized fields and its physical ABI ignores the value.
        let wrapper = types.intern(
            TypeShape::Adt {
                name: "IgnoredEnum".into(),
            },
            None,
        );
        types.record_semantic_aggregate(
            wrapper,
            SemanticAggregate {
                fields: vec![],
                variants: vec![vec![unit]],
            },
        );
        types.record_semantic_enum(wrapper, vec![0]);
        types.record_semantic_inhabited_variants(wrapper, vec![0]);
        let mut b = builder("ignore_return_observation", types, vec![], PassMode::Ignore);
        let ret = local(&mut b, wrapper);
        if initialized {
            let value = b.add_constant(Constant {
                ty: wrapper,
                kind: ConstantKind::ZeroSized,
            });
            assign(
                &mut b,
                Place::local(ret),
                Rvalue::Use(Operand::Constant(value)),
            );
        }
        b.finish_block(Terminator::Return, false);
        let graph = graph(&b.finish());
        let returned = graph
            .ids()
            .find(|&node| graph.op(node) == Opcode::Return)
            .unwrap();
        assert_eq!(graph.inputs(returned).len(), 2);
        assert_eq!(graph.rust_type(graph.input(returned, 1)), Some(wrapper));
        if initialized {
            assert_eq!(
                run(&graph, &[]),
                Ok(SemanticOutcome::Return(Value::Variant {
                    variant: 0,
                    fields: vec![Value::Aggregate(vec![])],
                }))
            );
        } else {
            assert!(matches!(
                run(&graph, &[]),
                Err(InterpretError::PoisonObserved(_))
            ));
        }
    }
}

#[test]
fn ignored_abi_argument_and_call_return_keep_source_enum_identity() {
    for indirect in [false, true] {
        let (mut types, _, _, _) = types();
        let unit = types.intern(TypeShape::Unit, None);
        let enumeration = types.intern(
            TypeShape::Adt {
                name: "IgnoredArgument".into(),
            },
            None,
        );
        types.record_semantic_aggregate(
            enumeration,
            SemanticAggregate {
                fields: vec![],
                variants: vec![vec![], vec![]],
            },
        );
        types.record_semantic_enum(enumeration, vec![3, 9]);
        types.record_semantic_inhabited_variants(enumeration, vec![1]);
        let pointer = types.intern(
            TypeShape::RawPointer {
                mutability: Mutability::Not,
                pointee: unit,
            },
            Some(SizeAlign { size: 8, align: 8 }),
        );
        let mut b = builder(
            "ignored_source_identity",
            types,
            vec![PassMode::Ignore],
            PassMode::Ignore,
        );
        let ret = local(&mut b, enumeration);
        let arg = local(&mut b, enumeration);
        b.set_arg_count(1);
        let abi = FunctionAbi {
            args: vec![PassMode::Ignore],
            ret: PassMode::Ignore,
            c_variadic: false,
        };
        let terminator = if indirect {
            let function = scalar(&mut b, pointer, 123);
            Terminator::CallIndirect {
                function,
                abi,
                args: vec![copy(arg)],
                destination: Place::local(ret),
                target: Some(BlockId(1)),
                unwind: UnwindAction::Unreachable,
            }
        } else {
            let callee = Operand::Constant(b.add_constant(Constant {
                ty: unit,
                kind: ConstantKind::Function(Callee::Function {
                    symbol: "ignored_identity".into(),
                    abi,
                }),
            }));
            Terminator::Call {
                callee,
                args: vec![copy(arg)],
                destination: Place::local(ret),
                target: Some(BlockId(1)),
                unwind: UnwindAction::Unreachable,
            }
        };
        b.finish_block(terminator, false);
        b.finish_block(Terminator::Return, false);
        let graph = graph(&b.finish());
        let call = graph
            .ids()
            .find(|&n| matches!(graph.op(n), Opcode::Call | Opcode::CallIndirect))
            .unwrap();
        let returned = graph.find_projection(call, 2).unwrap();
        let exit = graph
            .ids()
            .find(|&n| graph.op(n) == Opcode::Return)
            .unwrap();
        assert_eq!(graph.resolve(graph.input(exit, 1)), graph.resolve(returned));
        assert!(!graph.ids().any(|n| graph.op(n) == Opcode::RequireDefined));
        struct Identity {
            calls: usize,
        }
        impl kfrc_son::SemanticCallHandler for Identity {
            fn call(&mut self, _: &str, args: &[Value]) -> Result<Value, InterpretError> {
                self.calls += 1;
                Ok(args[0].clone())
            }
            fn call_indirect(
                &mut self,
                address: u64,
                args: &[Value],
            ) -> Result<Value, InterpretError> {
                assert_eq!(address, 123);
                self.call("indirect", args)
            }
        }
        let expected = Value::Variant {
            variant: 1,
            fields: vec![],
        };
        let mut calls = Identity { calls: 0 };
        assert_eq!(
            Interpreter::new(&graph)
                .run_values(std::slice::from_ref(&expected), &mut calls)
                .unwrap(),
            SemanticOutcome::Return(expected)
        );
        assert_eq!(calls.calls, 1);
        let mut calls = Identity { calls: 0 };
        assert!(matches!(
            Interpreter::new(&graph).run_values(&[Value::Poison], &mut calls),
            Err(InterpretError::PoisonObserved(_))
        ));
        assert_eq!(calls.calls, 0);
    }
}

#[test]
fn diverging_never_call_keeps_effect_and_unreachable_without_inventing_a_value() {
    let (mut types, _, _, _) = types();
    let unit = types.intern(TypeShape::Unit, None);
    let never = types.intern(TypeShape::Never, Some(SizeAlign { size: 0, align: 1 }));
    let mut b = builder("diverging_call", types, vec![], PassMode::Ignore);
    local(&mut b, unit);
    let destination = local(&mut b, never);
    let function = Operand::Constant(b.add_constant(Constant {
        ty: unit,
        kind: ConstantKind::Function(Callee::Function {
            symbol: "diverge".into(),
            abi: FunctionAbi {
                args: vec![],
                ret: PassMode::Ignore,
                c_variadic: false,
            },
        }),
    }));
    b.finish_block(
        Terminator::Call {
            callee: function,
            args: vec![],
            destination: Place::local(destination),
            target: None,
            unwind: UnwindAction::Unreachable,
        },
        false,
    );
    let graph = graph(&b.finish());
    let call = graph.ids().find(|&n| graph.op(n) == Opcode::Call).unwrap();
    assert!(graph.find_projection(call, 2).is_none());
    assert!(graph.ids().any(|n| graph.op(n) == Opcode::Unreachable));
    assert!(!graph.ids().any(|n| graph.op(n) == Opcode::Return));
    let mut trace = Vec::new();
    let result = Interpreter::new(&graph).run_values(&[], &mut |name: &str, _: &[Value]| {
        trace.push(name.to_owned());
        Err(InterpretError::UnknownCallee(
            "intentional-divergence-sentinel".into(),
        ))
    });
    assert_eq!(trace, ["diverge"]);
    assert_eq!(
        result,
        Err(InterpretError::UnknownCallee(
            "intentional-divergence-sentinel".into()
        ))
    );
}
