//! Shared source bodies for semantic and native DST place validation.
use kfrc_body::*;

fn local(b: &mut BodyBuilder, ty: TyId) -> LocalId {
    b.add_local(LocalDecl {
        ty,
        mutability: Mutability::Mut,
        name: None,
    })
}
fn copy(local: LocalId) -> Operand {
    Operand::Copy(Place::local(local))
}
fn assign(b: &mut BodyBuilder, destination: Place, value: Rvalue) {
    b.push_statement(StatementKind::Assign(destination, value));
}
fn constant(b: &mut BodyBuilder, ty: TyId, bits: u128) -> Operand {
    Operand::Constant(b.add_constant(Constant {
        ty,
        kind: ConstantKind::Scalar(bits),
    }))
}
fn pointer(types: &mut RecordedTypes, pointee: TyId, thin: TyId, word: TyId) -> TyId {
    let ty = types.intern(
        TypeShape::RawPointer {
            mutability: Mutability::Mut,
            pointee,
        },
        Some(SizeAlign { size: 16, align: 8 }),
    );
    types.record_aggregate(
        ty,
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
    ty
}

pub fn nested_tail_body(mark_tail: bool) -> (Body, TyId, TyId) {
    let mut types = RecordedTypes::new();
    let word = types.intern(
        TypeShape::Int {
            width: IntegerWidth::Pointer,
            signed: false,
        },
        Some(SizeAlign { size: 8, align: 8 }),
    );
    let byte = types.intern(
        TypeShape::Int {
            width: IntegerWidth::W8,
            signed: false,
        },
        Some(SizeAlign { size: 1, align: 1 }),
    );
    let thin = types.intern(
        TypeShape::RawPointer {
            mutability: Mutability::Mut,
            pointee: byte,
        },
        Some(SizeAlign { size: 8, align: 8 }),
    );
    let slice = types.intern(TypeShape::Slice { element: word }, None);
    let inner = types.intern(
        TypeShape::Adt {
            name: "NestedTail".into(),
        },
        None,
    );
    types.record_aggregate(
        inner,
        AggregateLayout {
            fields: vec![
                FieldLayout {
                    ty: word,
                    offset: 0,
                },
                FieldLayout {
                    ty: slice,
                    offset: 8,
                },
            ],
            variants: vec![],
        },
    );
    types.record_unsized(inner);
    if mark_tail {
        types.record_static_slice_tail(inner, 1);
    }
    let outer = types.intern(
        TypeShape::Adt {
            name: "OuterTail".into(),
        },
        None,
    );
    types.record_aggregate(
        outer,
        AggregateLayout {
            fields: vec![
                FieldLayout {
                    ty: word,
                    offset: 0,
                },
                FieldLayout {
                    ty: inner,
                    offset: 8,
                },
            ],
            variants: vec![],
        },
    );
    types.record_static_slice_tail(outer, 1);
    let wide_outer = pointer(&mut types, outer, thin, word);
    let wide_slice = pointer(&mut types, slice, thin, word);
    let mut b = BodyBuilder::new(
        "nested_tail",
        FunctionAbi {
            args: vec![],
            ret: PassMode::Direct,
            c_variadic: false,
        },
        types,
    );
    let ret = local(&mut b, word);
    let owner = local(&mut b, wide_outer);
    let middle = local(&mut b, wide_slice);
    let length = local(&mut b, word);
    let last = local(&mut b, word);
    let header = local(&mut b, word);
    let partial = local(&mut b, word);
    let bytes = [10u64, 20, 3, 5, 7, 9]
        .into_iter()
        .flat_map(u64::to_le_bytes)
        .collect();
    b.add_data_object(DataObject {
        symbol: "nested_tail_data".into(),
        binding: DataBinding::Local,
        bytes,
        align: 8,
        writable: true,
        thread_local: false,
        relocations: vec![],
    });
    let data = Operand::Constant(b.add_constant(Constant {
        ty: thin,
        kind: ConstantKind::Address {
            symbol: "nested_tail_data".into(),
            addend: 0,
        },
    }));
    let count = constant(&mut b, word, 4);
    assign(
        &mut b,
        Place::local(owner),
        Rvalue::Aggregate(AggregateKind::Tuple, vec![data, count]),
    );
    let prefix = Place {
        local: owner,
        projection: b.add_projection(vec![
            ProjectionElem::Deref,
            ProjectionElem::Field(FieldIndex(1), inner),
            ProjectionElem::Field(FieldIndex(0), word),
        ]),
    };
    let replacement = constant(&mut b, word, 33);
    assign(&mut b, prefix, Rvalue::Use(replacement));
    assign(
        &mut b,
        Place::local(header),
        Rvalue::Use(Operand::Copy(prefix)),
    );
    let tail = Place {
        local: owner,
        projection: b.add_projection(vec![
            ProjectionElem::Deref,
            ProjectionElem::Field(FieldIndex(1), inner),
            ProjectionElem::Field(FieldIndex(1), slice),
            ProjectionElem::Subslice {
                from: 1,
                to: 1,
                from_end: true,
                result_ty: slice,
            },
        ]),
    };
    assign(
        &mut b,
        Place::local(middle),
        Rvalue::RawPointer(Mutability::Mut, tail),
    );
    assign(
        &mut b,
        Place::local(length),
        Rvalue::UnaryOp(UnaryOp::PtrMetadata, copy(middle)),
    );
    let last_place = Place {
        local: middle,
        projection: b.add_projection(vec![
            ProjectionElem::Deref,
            ProjectionElem::ConstantIndex {
                offset: 1,
                min_length: 1,
                from_end: true,
            },
        ]),
    };
    assign(
        &mut b,
        Place::local(last),
        Rvalue::Use(Operand::Copy(last_place)),
    );
    assign(
        &mut b,
        Place::local(partial),
        Rvalue::BinaryOp(BinaryOp::Add, copy(header), copy(length)),
    );
    assign(
        &mut b,
        Place::local(ret),
        Rvalue::BinaryOp(BinaryOp::Add, copy(partial), copy(last)),
    );
    b.finish_block(Terminator::Return, false);
    (b.finish(), inner, slice)
}

pub fn string_tail_body() -> Body {
    let mut types = RecordedTypes::new();
    let word = types.intern(
        TypeShape::Int {
            width: IntegerWidth::Pointer,
            signed: false,
        },
        Some(SizeAlign { size: 8, align: 8 }),
    );
    let byte = types.intern(
        TypeShape::Int {
            width: IntegerWidth::W8,
            signed: false,
        },
        Some(SizeAlign { size: 1, align: 1 }),
    );
    let thin = types.intern(
        TypeShape::RawPointer {
            mutability: Mutability::Mut,
            pointee: byte,
        },
        Some(SizeAlign { size: 8, align: 8 }),
    );
    let string = types.intern(TypeShape::Str, None);
    let owner_ty = types.intern(
        TypeShape::Adt {
            name: "StringTail".into(),
        },
        None,
    );
    types.record_aggregate(
        owner_ty,
        AggregateLayout {
            fields: vec![
                FieldLayout {
                    ty: word,
                    offset: 0,
                },
                FieldLayout {
                    ty: string,
                    offset: 8,
                },
            ],
            variants: vec![],
        },
    );
    types.record_static_slice_tail(owner_ty, 1);
    let owner_ptr = pointer(&mut types, owner_ty, thin, word);
    let string_ref = types.intern(
        TypeShape::Reference {
            mutability: Mutability::Not,
            pointee: string,
        },
        Some(SizeAlign { size: 16, align: 8 }),
    );
    types.record_aggregate(
        string_ref,
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
    let mut b = BodyBuilder::new(
        "string_tail",
        FunctionAbi {
            args: vec![],
            ret: PassMode::Direct,
            c_variadic: false,
        },
        types,
    );
    let ret = local(&mut b, word);
    let owner = local(&mut b, owner_ptr);
    let reference = local(&mut b, string_ref);
    let length = local(&mut b, word);
    let data_ptr = local(&mut b, thin);
    let first = local(&mut b, word);
    b.add_data_object(DataObject {
        symbol: "string_tail_data".into(),
        binding: DataBinding::Local,
        bytes: [vec![0; 8], b"hello".to_vec()].concat(),
        align: 8,
        writable: false,
        thread_local: false,
        relocations: vec![],
    });
    let data = Operand::Constant(b.add_constant(Constant {
        ty: thin,
        kind: ConstantKind::Address {
            symbol: "string_tail_data".into(),
            addend: 0,
        },
    }));
    let five = constant(&mut b, word, 5);
    assign(
        &mut b,
        Place::local(owner),
        Rvalue::Aggregate(AggregateKind::Tuple, vec![data, five]),
    );
    let tail = Place {
        local: owner,
        projection: b.add_projection(vec![
            ProjectionElem::Deref,
            ProjectionElem::Field(FieldIndex(1), string),
        ]),
    };
    assign(
        &mut b,
        Place::local(reference),
        Rvalue::Ref(BorrowKind::Shared, tail),
    );
    assign(
        &mut b,
        Place::local(length),
        Rvalue::UnaryOp(UnaryOp::PtrMetadata, copy(reference)),
    );
    assign(
        &mut b,
        Place::local(data_ptr),
        Rvalue::Cast(CastKind::PtrToPtr, copy(reference), thin),
    );
    let first_byte = Place {
        local: data_ptr,
        projection: b.add_projection(vec![ProjectionElem::Deref]),
    };
    assign(
        &mut b,
        Place::local(first),
        Rvalue::Cast(CastKind::IntToInt, Operand::Copy(first_byte), word),
    );
    assign(
        &mut b,
        Place::local(ret),
        Rvalue::BinaryOp(BinaryOp::Add, copy(length), copy(first)),
    );
    b.finish_block(Terminator::Return, false);
    b.finish()
}
