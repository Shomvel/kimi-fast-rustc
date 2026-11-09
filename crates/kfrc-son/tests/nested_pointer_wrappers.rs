//! Nested pointer wrappers retain value semantics while their pointer widens.

use kfrc_body::*;
use kfrc_son::{Interpreter, Outcome, build_graph, verify_graph};

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

fn assign(builder: &mut BodyBuilder, local: LocalId, value: Rvalue) {
    builder.push_statement(StatementKind::Assign(Place::local(local), value));
}

fn wrapper(types: &mut RecordedTypes, name: &str, field: TyId, zst: TyId) -> TyId {
    let size = types.size_align(field).unwrap().size;
    let ty = types.intern(
        TypeShape::Adt { name: name.into() },
        Some(SizeAlign { size, align: 8 }),
    );
    types.record_aggregate(
        ty,
        AggregateLayout {
            fields: vec![
                FieldLayout {
                    ty: field,
                    offset: 0,
                },
                FieldLayout {
                    ty: zst,
                    offset: size,
                },
            ],
            variants: vec![],
        },
    );
    ty
}

#[test]
fn nested_wrappers_preserve_unsized_pointer_lanes_and_zst_fields() {
    let mut types = RecordedTypes::new();
    let word = types.intern(
        TypeShape::Int {
            width: IntegerWidth::W64,
            signed: false,
        },
        Some(SizeAlign { size: 8, align: 8 }),
    );
    let unit = types.intern(TypeShape::Unit, Some(SizeAlign { size: 0, align: 1 }));
    let zst = types.intern(
        TypeShape::Adt {
            name: "Allocator".into(),
        },
        Some(SizeAlign { size: 0, align: 1 }),
    );
    types.record_aggregate(
        zst,
        AggregateLayout { fields: vec![], variants: vec![] },
    );
    let array = types.intern(
        TypeShape::Array {
            element: word,
            length: 2,
        },
        Some(SizeAlign { size: 16, align: 8 }),
    );
    let slice = types.intern(TypeShape::Slice { element: word }, None);
    let thin = types.intern(
        TypeShape::RawPointer {
            mutability: Mutability::Not,
            pointee: array,
        },
        Some(SizeAlign { size: 8, align: 8 }),
    );
    let data = types.intern(
        TypeShape::RawPointer {
            mutability: Mutability::Not,
            pointee: unit,
        },
        Some(SizeAlign { size: 8, align: 8 }),
    );
    let fat = types.intern(
        TypeShape::RawPointer {
            mutability: Mutability::Not,
            pointee: slice,
        },
        Some(SizeAlign { size: 16, align: 8 }),
    );
    let lanes = [
        FieldLayout {
            ty: data,
            offset: 0,
        },
        FieldLayout {
            ty: word,
            offset: 8,
        },
    ];
    types.record_aggregate(
        fat,
        AggregateLayout {
            fields: lanes.to_vec(),
            variants: vec![],
        },
    );
    let thin_inner = wrapper(&mut types, "NonNull<[u64; 2]>", thin, zst);
    let thin_outer = wrapper(&mut types, "Box<[u64; 2]>", thin_inner, zst);
    let fat_inner = wrapper(&mut types, "NonNull<[u64]>", fat, zst);
    let fat_outer = wrapper(&mut types, "Box<[u64]>", fat_inner, zst);
    types.record_abi_pair(fat_outer, lanes);
    let abi = FunctionAbi {
        args: vec![],
        ret: PassMode::Direct,
        c_variadic: false,
    };
    let mut builder = BodyBuilder::new("nested_unsize", abi, types);
    let ret = local(&mut builder, word);
    let source_inner = local(&mut builder, thin_inner);
    let source = local(&mut builder, thin_outer);
    let pointer = local(&mut builder, data);
    let widened = local(&mut builder, fat);
    let target_inner = local(&mut builder, fat_inner);
    let target = local(&mut builder, fat_outer);
    let snapshot = local(&mut builder, fat_outer);
    let first = local(&mut builder, word);
    let sum = local(&mut builder, word);
    builder.add_data_object(DataObject {
        symbol: "two_words".into(),
        binding: DataBinding::Local,
        bytes: [20_u64.to_le_bytes(), 22_u64.to_le_bytes()].concat(),
        align: 8,
        writable: false,
        thread_local: false,
        relocations: vec![],
    });
    let address = Operand::Constant(builder.add_constant(Constant {
        ty: thin,
        kind: ConstantKind::Address {
            symbol: "two_words".into(),
            addend: 0,
        },
    }));
    let allocator = Operand::Constant(builder.add_constant(Constant {
        ty: zst,
        kind: ConstantKind::ZeroSized,
    }));
    assign(
        &mut builder,
        source_inner,
        Rvalue::Aggregate(AggregateKind::Tuple, vec![address, allocator]),
    );
    assign(
        &mut builder,
        source,
        Rvalue::Aggregate(AggregateKind::Tuple, vec![copy(source_inner), allocator]),
    );
    let nested = |builder: &mut BodyBuilder, outer: LocalId, inner: TyId, leaf: TyId| {
        let projection = builder.add_projection(vec![
            ProjectionElem::Field(FieldIndex(0), inner),
            ProjectionElem::Field(FieldIndex(0), leaf),
        ]);
        Operand::Copy(Place {
            local: outer,
            projection,
        })
    };
    let source_pointer = nested(&mut builder, source, thin_inner, thin);
    assign(
        &mut builder,
        pointer,
        Rvalue::Cast(CastKind::PtrToPtr, source_pointer, data),
    );
    let length = Operand::Constant(builder.add_constant(Constant {
        ty: word,
        kind: ConstantKind::Scalar(2),
    }));
    assign(
        &mut builder,
        widened,
        Rvalue::Aggregate(AggregateKind::Tuple, vec![copy(pointer), length]),
    );
    let phantom = builder.add_projection(vec![
        ProjectionElem::Field(FieldIndex(0), thin_inner),
        ProjectionElem::Field(FieldIndex(1), zst),
    ]);
    let phantom = Operand::Copy(Place {
        local: source,
        projection: phantom,
    });
    assign(
        &mut builder,
        target_inner,
        Rvalue::Aggregate(AggregateKind::Tuple, vec![copy(widened), phantom]),
    );
    let allocator_field = builder.add_projection(vec![ProjectionElem::Field(FieldIndex(1), zst)]);
    let allocator = Operand::Copy(Place {
        local: source,
        projection: allocator_field,
    });
    assign(
        &mut builder,
        target,
        Rvalue::Aggregate(AggregateKind::Tuple, vec![copy(target_inner), allocator]),
    );
    assign(&mut builder, snapshot, Rvalue::Use(copy(target)));
    // Reconstruction and copies must preserve both words independently of the
    // temporary used to construct the widened pointer.
    let null = Operand::Constant(builder.add_constant(Constant {
        ty: data,
        kind: ConstantKind::Scalar(0),
    }));
    let zero_length = Operand::Constant(builder.add_constant(Constant {
        ty: word,
        kind: ConstantKind::Scalar(0),
    }));
    assign(
        &mut builder,
        widened,
        Rvalue::Aggregate(AggregateKind::Tuple, vec![null, zero_length]),
    );
    for (index, output) in [(0, first), (1, sum)] {
        let projection = builder.add_projection(vec![
            ProjectionElem::Field(FieldIndex(0), fat_inner),
            ProjectionElem::Field(FieldIndex(0), fat),
            ProjectionElem::Deref,
            ProjectionElem::ConstantIndex {
                offset: index,
                min_length: 2,
                from_end: false,
            },
        ]);
        assign(
            &mut builder,
            output,
            Rvalue::Use(Operand::Copy(Place {
                local: snapshot,
                projection,
            })),
        );
    }
    assign(
        &mut builder,
        sum,
        Rvalue::BinaryOp(BinaryOp::Add, copy(first), copy(sum)),
    );
    let call = Operand::Constant(builder.add_constant(Constant {
        ty: unit,
        kind: ConstantKind::Function(Callee::Function {
            symbol: "consume_boxed_slice".into(),
            abi: FunctionAbi {
                args: vec![PassMode::Pair, PassMode::Direct, PassMode::Direct],
                ret: PassMode::Direct,
                c_variadic: false,
            },
        }),
    }));
    builder.finish_block(
        Terminator::Call {
            callee: call,
            args: vec![copy(snapshot), copy(pointer), copy(sum)],
            destination: Place::local(ret),
            target: Some(BlockId(1)),
            unwind: UnwindAction::Unreachable,
        },
        false,
    );
    builder.finish_block(Terminator::Return, false);
    let mut graph = build_graph(&builder.finish()).unwrap();
    verify_graph(&mut graph).unwrap();
    let mut calls = |symbol: &str, args: &[u64]| {
        assert_eq!(symbol, "consume_boxed_slice");
        assert_eq!(args.len(), 4);
        assert_eq!(args[0], args[2]);
        assert_eq!(args[1], 2);
        assert_eq!(args[3], 42);
        Ok(args[3])
    };
    assert_eq!(
        Interpreter::new(&layout_graph(&graph))
            .run(&[], &mut calls)
            .unwrap(),
        Outcome::Return(42)
    );
}

#[test]
fn wrapper_reconstruction_preserves_non_zst_field_when_offsets_change() {
    let mut types = RecordedTypes::new();
    let word = types.intern(
        TypeShape::Int {
            width: IntegerWidth::W64,
            signed: false,
        },
        Some(SizeAlign { size: 8, align: 8 }),
    );
    let unit = types.intern(TypeShape::Unit, Some(SizeAlign { size: 0, align: 1 }));
    let thin = types.intern(
        TypeShape::RawPointer {
            mutability: Mutability::Not,
            pointee: unit,
        },
        Some(SizeAlign { size: 8, align: 8 }),
    );
    let slice = types.intern(TypeShape::Slice { element: word }, None);
    let fat = types.intern(
        TypeShape::RawPointer {
            mutability: Mutability::Not,
            pointee: slice,
        },
        Some(SizeAlign { size: 16, align: 8 }),
    );
    types.record_aggregate(
        fat,
        AggregateLayout {
            fields: vec![
                FieldLayout {
                    ty: thin,
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
    let source_ty = types.intern(
        TypeShape::Adt {
            name: "Tagged<*const u64>".into(),
        },
        Some(SizeAlign { size: 16, align: 8 }),
    );
    types.record_aggregate(
        source_ty,
        AggregateLayout {
            fields: vec![
                FieldLayout {
                    ty: thin,
                    offset: 0,
                },
                FieldLayout {
                    ty: word,
                    offset: 8,
                },
                FieldLayout {
                    ty: unit,
                    offset: 16,
                },
            ],
            variants: vec![],
        },
    );
    let target_ty = types.intern(
        TypeShape::Adt {
            name: "Tagged<*const [u64]>".into(),
        },
        Some(SizeAlign { size: 24, align: 8 }),
    );
    types.record_aggregate(
        target_ty,
        AggregateLayout {
            fields: vec![
                FieldLayout { ty: fat, offset: 8 },
                FieldLayout {
                    ty: word,
                    offset: 0,
                },
                FieldLayout {
                    ty: unit,
                    offset: 24,
                },
            ],
            variants: vec![],
        },
    );
    let abi = FunctionAbi {
        args: vec![],
        ret: PassMode::Direct,
        c_variadic: false,
    };
    let mut builder = BodyBuilder::new("tagged_unsize", abi, types);
    let ret = local(&mut builder, word);
    let source = local(&mut builder, source_ty);
    let widened = local(&mut builder, fat);
    let target = local(&mut builder, target_ty);
    let snapshot = local(&mut builder, target_ty);
    let tag = local(&mut builder, word);
    let length = local(&mut builder, word);
    let pointer = Operand::Constant(builder.add_constant(Constant {
        ty: thin,
        kind: ConstantKind::Scalar(0x1230),
    }));
    let sentinel = Operand::Constant(builder.add_constant(Constant {
        ty: word,
        kind: ConstantKind::Scalar(40),
    }));
    let zst = Operand::Constant(builder.add_constant(Constant {
        ty: unit,
        kind: ConstantKind::ZeroSized,
    }));
    assign(
        &mut builder,
        source,
        Rvalue::Aggregate(AggregateKind::Tuple, vec![pointer, sentinel, zst]),
    );
    let mut source_field = |index, ty| {
        let projection = builder.add_projection(vec![ProjectionElem::Field(FieldIndex(index), ty)]);
        Operand::Copy(Place {
            local: source,
            projection,
        })
    };
    let pointer = source_field(0, thin);
    let sentinel = source_field(1, word);
    let zst = source_field(2, unit);
    let count = Operand::Constant(builder.add_constant(Constant {
        ty: word,
        kind: ConstantKind::Scalar(2),
    }));
    assign(
        &mut builder,
        widened,
        Rvalue::Aggregate(AggregateKind::Tuple, vec![pointer, count]),
    );
    assign(
        &mut builder,
        target,
        Rvalue::Aggregate(AggregateKind::Tuple, vec![copy(widened), sentinel, zst]),
    );
    assign(&mut builder, snapshot, Rvalue::Use(copy(target)));
    let target_tag = builder.add_projection(vec![ProjectionElem::Field(FieldIndex(1), word)]);
    let overwrite = Operand::Constant(builder.add_constant(Constant {
        ty: word,
        kind: ConstantKind::Scalar(99),
    }));
    builder.push_statement(StatementKind::Assign(
        Place {
            local: target,
            projection: target_tag,
        },
        Rvalue::Use(overwrite),
    ));
    assign(
        &mut builder,
        tag,
        Rvalue::Use(Operand::Copy(Place {
            local: snapshot,
            projection: target_tag,
        })),
    );
    let target_pointer = builder.add_projection(vec![ProjectionElem::Field(FieldIndex(0), fat)]);
    assign(
        &mut builder,
        length,
        Rvalue::UnaryOp(
            UnaryOp::PtrMetadata,
            Operand::Copy(Place {
                local: snapshot,
                projection: target_pointer,
            }),
        ),
    );
    assign(
        &mut builder,
        ret,
        Rvalue::BinaryOp(BinaryOp::Add, copy(tag), copy(length)),
    );
    builder.finish_block(Terminator::Return, false);
    let mut graph = build_graph(&builder.finish()).unwrap();
    verify_graph(&mut graph).unwrap();
    let mut calls = |_: &str, _: &[u64]| unreachable!();
    assert_eq!(
        Interpreter::new(&layout_graph(&graph))
            .run(&[], &mut calls)
            .unwrap(),
        Outcome::Return(42)
    );
}

/// Exercise the unique value pipeline before interpreting its physical ABI.
fn layout_graph(graph: &kfrc_son::FunctionGraph) -> kfrc_son::FunctionGraph {
    let mut graph = graph.clone();
    kfrc_son::verify_graph(&mut graph).unwrap();
    kfrc_son::lower_layout(&mut graph).unwrap();
    kfrc_son::verify_graph(&mut graph).unwrap();
    graph
}
