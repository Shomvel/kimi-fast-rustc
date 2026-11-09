//! Union input fixtures are explicitly outside the semantic PoC scope.

use kfrc_body::*;
use kfrc_son::{BuildError, build_graph};

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

#[test]
fn union_payload_and_copy_fixture_is_explicitly_rejected() {
    let mut types = RecordedTypes::new();
    let word = types.intern(
        TypeShape::Int {
            width: IntegerWidth::W64,
            signed: false,
        },
        Some(SizeAlign { size: 8, align: 8 }),
    );
    let unit = types.intern(TypeShape::Unit, Some(SizeAlign { size: 0, align: 1 }));
    let boolean = types.intern(TypeShape::Bool, Some(SizeAlign { size: 1, align: 1 }));
    let pair = types.intern(
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
    let union = types.intern(
        TypeShape::Adt {
            name: "MaybeUninitPair".into(),
        },
        Some(SizeAlign { size: 16, align: 8 }),
    );
    types.record_union(
        union,
        vec![
            FieldLayout {
                ty: unit,
                offset: 0,
            },
            FieldLayout {
                ty: pair,
                offset: 0,
            },
            FieldLayout {
                ty: boolean,
                offset: 0,
            },
        ],
    );
    let abi = FunctionAbi {
        args: vec![],
        ret: PassMode::Direct,
        c_variadic: false,
    };
    let mut builder = BodyBuilder::new("copy_union", abi, types);
    let ret = local(&mut builder, word);
    let payload = local(&mut builder, pair);
    let original = local(&mut builder, union);
    let snapshot = local(&mut builder, union);
    let first = local(&mut builder, word);
    let second = local(&mut builder, word);
    // The first byte is invalid as a bool, but valid in the selected pair.
    let first_value = Operand::Constant(builder.add_constant(Constant {
        ty: word,
        kind: ConstantKind::Scalar(0x1122_3344_5566_7780),
    }));
    let second_value = Operand::Constant(builder.add_constant(Constant {
        ty: word,
        kind: ConstantKind::Scalar(42),
    }));
    assign(
        &mut builder,
        payload,
        Rvalue::Aggregate(AggregateKind::Tuple, vec![first_value, second_value]),
    );
    assign(
        &mut builder,
        original,
        Rvalue::Aggregate(
            AggregateKind::Union {
                ty: union,
                field: FieldIndex(1),
            },
            vec![copy(payload)],
        ),
    );
    assign(&mut builder, snapshot, Rvalue::Use(copy(original)));
    let overwrite = Operand::Constant(builder.add_constant(Constant {
        ty: boolean,
        kind: ConstantKind::Scalar(1),
    }));
    assign(
        &mut builder,
        original,
        Rvalue::Aggregate(
            AggregateKind::Union {
                ty: union,
                field: FieldIndex(2),
            },
            vec![overwrite],
        ),
    );
    for (index, destination) in [(0, first), (1, second)] {
        let projection = builder.add_projection(vec![
            ProjectionElem::Field(FieldIndex(1), pair),
            ProjectionElem::Field(FieldIndex(index), word),
        ]);
        assign(
            &mut builder,
            destination,
            Rvalue::Use(Operand::Copy(Place {
                local: snapshot,
                projection,
            })),
        );
    }
    assign(
        &mut builder,
        ret,
        Rvalue::BinaryOp(BinaryOp::Add, copy(first), copy(second)),
    );
    builder.finish_block(Terminator::Return, false);
    assert!(matches!(
        build_graph(&builder.finish()),
        Err(BuildError::Unsupported { what, .. }) if what.to_lowercase().contains("union")
    ));
}

#[test]
fn zero_sized_union_constructor_is_explicitly_rejected() {
    let mut types = RecordedTypes::new();
    let unit = types.intern(TypeShape::Unit, Some(SizeAlign { size: 0, align: 1 }));
    let union = types.intern(
        TypeShape::Adt {
            name: "EmptyUnion".into(),
        },
        Some(SizeAlign { size: 0, align: 1 }),
    );
    types.record_union(
        union,
        vec![FieldLayout {
            ty: unit,
            offset: 0,
        }],
    );
    let abi = FunctionAbi {
        args: vec![],
        ret: PassMode::Ignore,
        c_variadic: false,
    };
    let mut builder = BodyBuilder::new("empty_union", abi, types);
    let ret = local(&mut builder, unit);
    let value = local(&mut builder, union);
    let empty = Operand::Constant(builder.add_constant(Constant {
        ty: unit,
        kind: ConstantKind::ZeroSized,
    }));
    assign(
        &mut builder,
        value,
        Rvalue::Aggregate(
            AggregateKind::Union {
                ty: union,
                field: FieldIndex(0),
            },
            vec![empty],
        ),
    );
    assign(&mut builder, ret, Rvalue::Use(empty));
    builder.finish_block(Terminator::Return, false);
    assert!(matches!(
        build_graph(&builder.finish()),
        Err(BuildError::Unsupported { what, .. }) if what.to_lowercase().contains("union")
    ));
}
