//! Zero-sized aggregates retain structural arity and surrounding call effects.

use kfrc_body::*;
use kfrc_son::{BuildError, Interpreter, Opcode, Outcome, build_graph, verify_graph};

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

fn zst(types: &mut RecordedTypes, name: &str, fields: Vec<TyId>) -> TyId {
    let ty = types.intern(
        TypeShape::Adt { name: name.into() },
        Some(SizeAlign { size: 0, align: 1 }),
    );
    types.record_aggregate(
        ty,
        AggregateLayout {
            fields: fields
                .into_iter()
                .map(|ty| FieldLayout { ty, offset: 0 })
                .collect(),
            variants: vec![],
        },
    );
    ty
}

fn function(
    builder: &mut BodyBuilder,
    ty: TyId,
    symbol: &str,
    args: Vec<PassMode>,
    ret: PassMode,
) -> Operand {
    Operand::Constant(builder.add_constant(Constant {
        ty,
        kind: ConstantKind::Function(Callee::Function {
            symbol: symbol.into(),
            abi: FunctionAbi {
                args,
                ret,
                c_variadic: false,
            },
        }),
    }))
}

#[test]
fn zst_tuple_closure_struct_array_and_enum_keep_fields_and_call_effects() {
    let mut types = RecordedTypes::new();
    let unit = types.intern(TypeShape::Unit, Some(SizeAlign { size: 0, align: 1 }));
    let word = types.intern(
        TypeShape::Int {
            width: IntegerWidth::W64,
            signed: false,
        },
        Some(SizeAlign { size: 8, align: 8 }),
    );
    let fragment = zst(&mut types, "Fragment", vec![]);
    let tuple_ty = types.intern(
        TypeShape::Tuple(vec![unit, fragment]),
        Some(SizeAlign { size: 0, align: 1 }),
    );
    types.record_aggregate(
        tuple_ty,
        AggregateLayout {
            fields: vec![
                FieldLayout {
                    ty: unit,
                    offset: 0,
                },
                FieldLayout {
                    ty: fragment,
                    offset: 0,
                },
            ],
            variants: vec![],
        },
    );
    let closure_ty = zst(
        &mut types,
        "CapturesFragmentAndTuple",
        vec![fragment, tuple_ty],
    );
    let struct_ty = zst(&mut types, "ZstRecord", vec![unit, closure_ty]);
    let array_ty = types.intern(
        TypeShape::Array {
            element: tuple_ty,
            length: 2,
        },
        Some(SizeAlign { size: 0, align: 1 }),
    );
    let enum_ty = types.intern(
        TypeShape::Adt {
            name: "SingleZstVariant".into(),
        },
        Some(SizeAlign { size: 0, align: 1 }),
    );
    types.record_aggregate(
        enum_ty,
        AggregateLayout {
            fields: vec![],
            variants: vec![vec![
                FieldLayout {
                    ty: struct_ty,
                    offset: 0,
                },
                FieldLayout {
                    ty: array_ty,
                    offset: 0,
                },
            ]],
        },
    );
    types.record_semantic_enum(enum_ty, vec![0]);
    types.enums.insert(
        enum_ty,
        EnumLayout {
            tag: None,
            encoding: TagEncoding::Single {
                variant: 0,
                discriminant: 0,
            },
        },
    );
    let abi = FunctionAbi {
        args: vec![],
        ret: PassMode::Direct,
        c_variadic: false,
    };
    let mut builder = BodyBuilder::new("all_zst_aggregates", abi, types);
    let ret = local(&mut builder, word);
    let value = local(&mut builder, fragment);
    let tuple = local(&mut builder, tuple_ty);
    let closure = local(&mut builder, closure_ty);
    let record = local(&mut builder, struct_ty);
    let array = local(&mut builder, array_ty);
    let enumeration = local(&mut builder, enum_ty);
    let make = function(
        &mut builder,
        unit,
        "make_fragment",
        vec![],
        PassMode::Ignore,
    );
    builder.finish_block(
        Terminator::Call {
            callee: make,
            args: vec![],
            destination: Place::local(value),
            target: Some(BlockId(1)),
            unwind: UnwindAction::Unreachable,
        },
        false,
    );
    let empty = Operand::Constant(builder.add_constant(Constant {
        ty: unit,
        kind: ConstantKind::ZeroSized,
    }));
    assign(
        &mut builder,
        tuple,
        Rvalue::Aggregate(AggregateKind::Tuple, vec![empty, copy(value)]),
    );
    assign(
        &mut builder,
        closure,
        Rvalue::Aggregate(
            AggregateKind::Closure(closure_ty),
            vec![copy(value), copy(tuple)],
        ),
    );
    assign(
        &mut builder,
        record,
        Rvalue::Aggregate(
            AggregateKind::Adt {
                ty: struct_ty,
                variant: VariantIndex(0),
            },
            vec![empty, copy(closure)],
        ),
    );
    assign(
        &mut builder,
        array,
        Rvalue::Aggregate(
            AggregateKind::Array(tuple_ty),
            vec![copy(tuple), copy(tuple)],
        ),
    );
    assign(
        &mut builder,
        enumeration,
        Rvalue::Aggregate(
            AggregateKind::Adt {
                ty: enum_ty,
                variant: VariantIndex(0),
            },
            vec![copy(record), copy(array)],
        ),
    );
    let consume = function(
        &mut builder,
        unit,
        "consume_zsts",
        vec![PassMode::Ignore; 5],
        PassMode::Direct,
    );
    builder.finish_block(
        Terminator::Call {
            callee: consume,
            args: vec![
                copy(tuple),
                copy(closure),
                copy(record),
                copy(array),
                copy(enumeration),
            ],
            destination: Place::local(ret),
            target: Some(BlockId(2)),
            unwind: UnwindAction::Unreachable,
        },
        false,
    );
    builder.finish_block(Terminator::Return, false);
    let mut graph = build_graph(&builder.finish()).unwrap();
    verify_graph(&mut graph).unwrap();
    assert!(
        !graph
            .ids()
            .any(|node| matches!(graph.op(node), Opcode::Load | Opcode::Store))
    );
    let mut seen = Vec::new();
    let mut calls = |symbol: &str, args: &[u64]| {
        assert!(args.is_empty());
        seen.push(symbol.to_owned());
        Ok(42)
    };
    assert_eq!(
        Interpreter::new(&layout_graph(&graph))
            .run(&[], &mut calls)
            .unwrap(),
        Outcome::Return(42)
    );
    assert_eq!(seen, ["make_fragment", "consume_zsts"]);
}

#[test]
fn zst_field_arity_is_checked_instead_of_discarded_by_size() {
    for category in ["tuple", "closure", "struct", "array"] {
        let mut types = RecordedTypes::new();
        let unit = types.intern(TypeShape::Unit, Some(SizeAlign { size: 0, align: 1 }));
        let pair_ty = if category == "tuple" {
            let ty = types.intern(
                TypeShape::Tuple(vec![unit, unit]),
                Some(SizeAlign { size: 0, align: 1 }),
            );
            types.record_aggregate(
                ty,
                AggregateLayout {
                    fields: vec![
                        FieldLayout {
                            ty: unit,
                            offset: 0
                        };
                        2
                    ],
                    variants: vec![],
                },
            );
            ty
        } else if category == "array" {
            types.intern(
                TypeShape::Array {
                    element: unit,
                    length: 2,
                },
                Some(SizeAlign { size: 0, align: 1 }),
            )
        } else {
            zst(&mut types, category, vec![unit, unit])
        };
        let kind = match category {
            "tuple" => AggregateKind::Tuple,
            "array" => AggregateKind::Array(unit),
            "closure" => AggregateKind::Closure(pair_ty),
            _ => AggregateKind::Adt {
                ty: pair_ty,
                variant: VariantIndex(0),
            },
        };
        let abi = FunctionAbi {
            args: vec![],
            ret: PassMode::Ignore,
            c_variadic: false,
        };
        let mut builder = BodyBuilder::new(category, abi, types);
        let _ret = local(&mut builder, unit);
        let value = local(&mut builder, pair_ty);
        assign(&mut builder, value, Rvalue::Aggregate(kind, vec![]));
        builder.finish_block(Terminator::Return, false);
        let error = build_graph(&builder.finish()).unwrap_err();
        let expected = if category == "array" {
            "aggregate operand count disagrees with array length"
        } else {
            "aggregate operand count disagrees with semantic fields"
        };
        assert!(
            matches!(&error, BuildError::Unsupported { what, .. } if what == expected),
            "{category}: {error:?}"
        );
    }
}

/// Exercise the unique value pipeline before interpreting its physical ABI.
fn layout_graph(graph: &kfrc_son::FunctionGraph) -> kfrc_son::FunctionGraph {
    let mut graph = graph.clone();
    kfrc_son::verify_graph(&mut graph).unwrap();
    kfrc_son::lower_layout(&mut graph).unwrap();
    kfrc_son::verify_graph(&mut graph).unwrap();
    graph
}
