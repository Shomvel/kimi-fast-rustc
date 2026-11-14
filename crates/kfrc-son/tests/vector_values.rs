//! Retain SIMD Body fixtures as explicit unsupported-input coverage.

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

fn vector_types() -> (RecordedTypes, TyId, TyId, TyId) {
    let mut types = RecordedTypes::new();
    let word = types.intern(
        TypeShape::Int {
            width: IntegerWidth::W64,
            signed: false,
        },
        Some(SizeAlign { size: 8, align: 8 }),
    );
    let array = types.intern(
        TypeShape::Array {
            element: word,
            length: 2,
        },
        Some(SizeAlign { size: 16, align: 8 }),
    );
    let vector = types.intern(
        TypeShape::Adt {
            name: "__m128i".into(),
        },
        Some(SizeAlign {
            size: 16,
            align: 16,
        }),
    );
    // The logical struct still has one array field. Physical SIMD lanes must
    // not replace it with two fields, or constructors change arity.
    types.record_aggregate(
        vector,
        AggregateLayout {
            fields: vec![FieldLayout {
                ty: array,
                offset: 0,
            }],
            variants: vec![],
        },
    );
    types.record_vector(
        vector,
        VectorLayout {
            element: word,
            lanes: 2,
        },
    );
    (types, word, array, vector)
}

fn assert_vector_rejected(builder: BodyBuilder) {
    assert!(matches!(
        build_graph(&builder.finish()),
        Err(BuildError::Unsupported { what, .. })
            if what.to_lowercase().contains("vector") || what.contains("SIMD")
    ));
}

#[test]
fn vector_parameter_snapshot_fixture_is_rejected() {
    let (types, word, array, vector) = vector_types();
    let abi = FunctionAbi {
        args: vec![PassMode::Vector { bytes: 16 }, PassMode::Direct],
        ret: PassMode::Vector { bytes: 16 },
        c_variadic: false,
    };
    let mut builder = BodyBuilder::new("vector_xor", abi, types);
    let ret = local(&mut builder, vector);
    let input = local(&mut builder, vector);
    let mask = local(&mut builder, word);
    builder.set_arg_count(2);
    let snapshot = local(&mut builder, vector);
    let lanes = [local(&mut builder, word), local(&mut builder, word)];
    let output_array = local(&mut builder, array);
    assign(&mut builder, snapshot, Rvalue::Use(copy(input)));
    for (index, destination) in lanes.into_iter().enumerate() {
        let projection = builder.add_projection(vec![
            ProjectionElem::Field(FieldIndex(0), array),
            ProjectionElem::ConstantIndex {
                offset: index as u64,
                min_length: 2,
                from_end: false,
            },
        ]);
        let source = Operand::Copy(Place {
            local: snapshot,
            projection,
        });
        assign(
            &mut builder,
            destination,
            Rvalue::BinaryOp(BinaryOp::BitXor, source, copy(mask)),
        );
    }
    assign(
        &mut builder,
        output_array,
        Rvalue::Aggregate(
            AggregateKind::Array(word),
            lanes.into_iter().map(copy).collect(),
        ),
    );
    assign(
        &mut builder,
        ret,
        Rvalue::Aggregate(
            AggregateKind::Adt {
                ty: vector,
                variant: VariantIndex(0),
            },
            vec![copy(output_array)],
        ),
    );
    builder.finish_block(Terminator::Return, false);
    assert_vector_rejected(builder);
}

#[test]
fn direct_vector_call_fixture_is_rejected() {
    let (types, word, _, vector) = vector_types();
    let abi = FunctionAbi {
        args: vec![PassMode::Vector { bytes: 16 }],
        ret: PassMode::Vector { bytes: 16 },
        c_variadic: false,
    };
    let mut builder = BodyBuilder::new("vector_caller", abi, types);
    let ret = local(&mut builder, vector);
    let arg = local(&mut builder, vector);
    builder.set_arg_count(1);
    let number = Operand::Constant(builder.add_constant(Constant {
        ty: word,
        kind: ConstantKind::Scalar(7),
    }));
    let callee = Operand::Constant(builder.add_constant(Constant {
        ty: word,
        kind: ConstantKind::Function(Callee::Function {
            symbol: "vector_transform".into(),
            abi: FunctionAbi {
                args: vec![PassMode::Vector { bytes: 16 }, PassMode::Direct],
                ret: PassMode::Vector { bytes: 16 },
                c_variadic: false,
            },
        }),
    }));
    builder.finish_block(
        Terminator::Call {
            callee,
            args: vec![copy(arg), number],
            destination: Place::local(ret),
            target: Some(BlockId(1)),
            unwind: UnwindAction::Unreachable,
        },
        false,
    );
    builder.finish_block(Terminator::Return, false);
    assert_vector_rejected(builder);
}

#[test]
fn unsupported_vector_width_and_u128_scalar_remain_rejected() {
    for scalar in [false, true] {
        let (mut types, _, _, vector) = vector_types();
        let ty = if scalar {
            types.intern(
                TypeShape::Int {
                    width: IntegerWidth::W128,
                    signed: false,
                },
                Some(SizeAlign {
                    size: 16,
                    align: 16,
                }),
            )
        } else {
            vector
        };
        let abi = FunctionAbi {
            args: vec![if scalar {
                PassMode::Direct
            } else {
                PassMode::Vector { bytes: 32 }
            }],
            ret: PassMode::Ignore,
            c_variadic: false,
        };
        let mut builder = BodyBuilder::new("unsupported_wide", abi, types);
        let _ret = local(&mut builder, ty);
        let _arg = local(&mut builder, ty);
        builder.set_arg_count(1);
        builder.finish_block(Terminator::Return, false);
        assert!(matches!(
            build_graph(&builder.finish()),
            Err(BuildError::Unsupported { .. })
        ));
    }
}

#[test]
fn local_vectors_are_rejected_at_every_recorded_width() {
    for size in [1, 2, 4, 8, 64] {
        let mut types = RecordedTypes::new();
        let byte = types.intern(
            TypeShape::Int {
                width: IntegerWidth::W8,
                signed: false,
            },
            Some(SizeAlign { size: 1, align: 1 }),
        );
        let array = types.intern(
            TypeShape::Array {
                element: byte,
                length: size,
            },
            Some(SizeAlign { size, align: 1 }),
        );
        let vector = types.intern(
            TypeShape::Adt {
                name: format!("Simd<u8,{size}>"),
            },
            Some(SizeAlign { size, align: size }),
        );
        types.record_aggregate(
            vector,
            AggregateLayout {
                fields: vec![FieldLayout {
                    ty: array,
                    offset: 0,
                }],
                variants: vec![],
            },
        );
        types.record_vector(
            vector,
            VectorLayout {
                element: byte,
                lanes: size as u32,
            },
        );
        let abi = FunctionAbi {
            args: vec![],
            ret: PassMode::Direct,
            c_variadic: false,
        };
        let mut builder = BodyBuilder::new("local_simd_copy", abi, types);
        let ret = local(&mut builder, byte);
        let data = local(&mut builder, array);
        let source = local(&mut builder, vector);
        let snapshot = local(&mut builder, vector);
        let values = (0..size)
            .map(|index| {
                Operand::Constant(builder.add_constant(Constant {
                    ty: byte,
                    kind: ConstantKind::Scalar(u128::from(index + 1)),
                }))
            })
            .collect();
        assign(
            &mut builder,
            data,
            Rvalue::Aggregate(AggregateKind::Array(byte), values),
        );
        assign(
            &mut builder,
            source,
            Rvalue::Aggregate(
                AggregateKind::Adt {
                    ty: vector,
                    variant: VariantIndex(0),
                },
                vec![copy(data)],
            ),
        );
        assign(&mut builder, snapshot, Rvalue::Use(copy(source)));
        let last = builder.add_projection(vec![
            ProjectionElem::Field(FieldIndex(0), array),
            ProjectionElem::ConstantIndex {
                offset: size - 1,
                min_length: size,
                from_end: false,
            },
        ]);
        let zero = Operand::Constant(builder.add_constant(Constant {
            ty: byte,
            kind: ConstantKind::Scalar(0),
        }));
        builder.push_statement(StatementKind::Assign(
            Place {
                local: source,
                projection: last,
            },
            Rvalue::Use(zero),
        ));
        assign(
            &mut builder,
            ret,
            Rvalue::Use(Operand::Copy(Place {
                local: snapshot,
                projection: last,
            })),
        );
        builder.finish_block(Terminator::Return, false);
        assert_vector_rejected(builder);
    }
}
