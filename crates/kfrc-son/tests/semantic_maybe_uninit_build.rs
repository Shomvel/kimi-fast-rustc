//! Producer-marked MaybeUninit remains a raw container until typed decoding.
use kfrc_body::*;
use kfrc_son::{
    FunctionGraph, InterpretError, Interpreter, Opcode, SemanticOutcome, Value, build_graph,
    verify_graph,
};

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
fn types(marked: bool) -> (RecordedTypes, TyId, TyId, TyId) {
    let mut types = RecordedTypes::new();
    let unit = types.intern(TypeShape::Unit, Some(SizeAlign { size: 0, align: 1 }));
    let boolean = types.intern(TypeShape::Bool, Some(SizeAlign { size: 1, align: 1 }));
    let wrapper = types.intern(
        TypeShape::Adt {
            name: "MaybeUninit<bool>".into(),
        },
        Some(SizeAlign { size: 1, align: 1 }),
    );
    types.record_union(
        wrapper,
        vec![
            FieldLayout {
                ty: unit,
                offset: 0,
            },
            FieldLayout {
                ty: boolean,
                offset: 0,
            },
        ],
    );
    if marked {
        types.record_maybe_uninit(wrapper, boolean);
    }
    (types, unit, boolean, wrapper)
}
fn builder(types: RecordedTypes, args: usize) -> BodyBuilder {
    BodyBuilder::new(
        "maybe_uninit_body",
        FunctionAbi {
            args: vec![PassMode::Direct; args],
            ret: PassMode::Direct,
            c_variadic: false,
        },
        types,
    )
}
fn graph(b: BodyBuilder) -> FunctionGraph {
    let mut graph = build_graph(&b.finish()).unwrap();
    verify_graph(&mut graph).unwrap();
    graph
}
fn run(graph: &FunctionGraph, args: &[Value]) -> Result<SemanticOutcome, InterpretError> {
    Interpreter::new(graph).run_values(args, &mut |name: &str, _: &[Value]| {
        Err(InterpretError::UnknownCallee(name.into()))
    })
}
fn value_field(b: &mut BodyBuilder, wrapper: LocalId, boolean: TyId) -> Place {
    Place {
        local: wrapper,
        projection: b.add_projection(vec![ProjectionElem::Field(FieldIndex(1), boolean)]),
    }
}

#[test]
fn marked_union_constructors_preserve_initialized_and_uninitialized_bytes() {
    for initialized in [false, true] {
        let (types, unit, boolean, wrapper) = types(true);
        let mut b = builder(types, 0);
        let ret = local(&mut b, wrapper);
        let (field, ty, kind) = if initialized {
            (1, boolean, ConstantKind::Scalar(1))
        } else {
            (0, unit, ConstantKind::ZeroSized)
        };
        let operand = Operand::Constant(b.add_constant(Constant { ty, kind }));
        assign(
            &mut b,
            Place::local(ret),
            Rvalue::Aggregate(
                AggregateKind::Union {
                    ty: wrapper,
                    field: FieldIndex(field),
                },
                vec![operand],
            ),
        );
        b.finish_block(Terminator::Return, false);
        let graph = graph(b);
        let SemanticOutcome::Return(Value::Uninit { bytes }) = run(&graph, &[]).unwrap() else {
            panic!("expected raw MaybeUninit container");
        };
        assert_eq!(&*bytes, &[if initialized { Some(1) } else { None }]);
        assert!(
            !graph
                .ids()
                .any(|n| graph.op(n) == Opcode::Aggregate && graph.rust_type(n) == Some(wrapper))
        );
    }
}

#[test]
fn ordinary_union_with_maybe_uninit_name_is_still_rejected() {
    let (types, _, _, wrapper) = types(false);
    let mut b = builder(types, 1);
    let ret = local(&mut b, wrapper);
    let arg = local(&mut b, wrapper);
    b.set_arg_count(1);
    assign(&mut b, Place::local(ret), Rvalue::Use(copy(arg)));
    b.finish_block(Terminator::Return, false);
    assert!(
        build_graph(&b.finish())
            .unwrap_err()
            .to_string()
            .contains("union")
    );
}

#[test]
fn typed_field_read_rejects_invalid_bool_but_container_copy_accepts_it() {
    let (types, _, boolean, wrapper) = types(true);
    let mut b = builder(types.clone(), 1);
    let ret = local(&mut b, wrapper);
    let arg = local(&mut b, wrapper);
    b.set_arg_count(1);
    assign(&mut b, Place::local(ret), Rvalue::Use(copy(arg)));
    b.finish_block(Terminator::Return, false);
    let raw_graph = graph(b);
    let invalid = Value::Uninit {
        bytes: std::sync::Arc::new(vec![Some(2)]),
    };
    assert_eq!(
        run(&raw_graph, std::slice::from_ref(&invalid)).unwrap(),
        SemanticOutcome::Return(invalid.clone())
    );
    let mut b = builder(types, 1);
    let ret = local(&mut b, boolean);
    let arg = local(&mut b, wrapper);
    b.set_arg_count(1);
    let field = value_field(&mut b, arg, boolean);
    assign(&mut b, Place::local(ret), Rvalue::Use(Operand::Copy(field)));
    b.finish_block(Terminator::Return, false);
    let typed_graph = graph(b);
    assert!(
        typed_graph
            .ids()
            .any(|n| typed_graph.op(n) == Opcode::TransmuteValue)
    );
    assert!(matches!(
        run(&typed_graph, &[invalid]),
        Err(InterpretError::UndefinedBehaviour(_))
    ));
    assert!(matches!(
        run(
            &typed_graph,
            &[Value::Uninit {
                bytes: std::sync::Arc::new(vec![None])
            }]
        ),
        Err(InterpretError::PoisonObserved(_))
    ));
    assert_eq!(
        run(
            &typed_graph,
            &[Value::Uninit {
                bytes: std::sync::Arc::new(vec![Some(1)])
            }]
        )
        .unwrap(),
        SemanticOutcome::Return(Value::Scalar(1))
    );
}

#[test]
fn whole_value_field_write_initializes_container_without_reading_old_bytes() {
    let (types, _, boolean, wrapper) = types(true);
    let mut b = builder(types, 0);
    let ret = local(&mut b, boolean);
    let value = local(&mut b, wrapper);
    let field = value_field(&mut b, value, boolean);
    let yes = Operand::Constant(b.add_constant(Constant {
        ty: boolean,
        kind: ConstantKind::Scalar(1),
    }));
    assign(&mut b, field, Rvalue::Use(yes));
    assign(&mut b, Place::local(ret), Rvalue::Use(Operand::Copy(field)));
    b.finish_block(Terminator::Return, false);
    let graph = graph(b);
    assert_eq!(
        run(&graph, &[]).unwrap(),
        SemanticOutcome::Return(Value::Scalar(1))
    );
}

#[test]
fn partial_payload_write_preserves_uninitialized_sibling_bytes() {
    let (mut types, unit, boolean, _) = types(true);
    let pair = types.intern(
        TypeShape::Tuple(vec![boolean, boolean]),
        Some(SizeAlign { size: 2, align: 1 }),
    );
    types.record_aggregate(
        pair,
        AggregateLayout {
            fields: vec![
                FieldLayout {
                    ty: boolean,
                    offset: 0,
                },
                FieldLayout {
                    ty: boolean,
                    offset: 1,
                },
            ],
            variants: vec![],
        },
    );
    let wrapper = types.intern(
        TypeShape::Adt {
            name: "MaybeUninit<(bool,bool)>".into(),
        },
        Some(SizeAlign { size: 2, align: 1 }),
    );
    types.record_union(
        wrapper,
        vec![
            FieldLayout {
                ty: unit,
                offset: 0,
            },
            FieldLayout {
                ty: pair,
                offset: 0,
            },
        ],
    );
    types.record_maybe_uninit(wrapper, pair);
    let mut b = builder(types, 0);
    let ret = local(&mut b, wrapper);
    let value = local(&mut b, wrapper);
    let first = Place {
        local: value,
        projection: b.add_projection(vec![
            ProjectionElem::Field(FieldIndex(1), pair),
            ProjectionElem::Field(FieldIndex(0), boolean),
        ]),
    };
    let yes = Operand::Constant(b.add_constant(Constant {
        ty: boolean,
        kind: ConstantKind::Scalar(1),
    }));
    assign(&mut b, first, Rvalue::Use(yes));
    assign(&mut b, Place::local(ret), Rvalue::Use(copy(value)));
    b.finish_block(Terminator::Return, false);
    let graph = graph(b);
    assert!(graph.ids().any(|n| graph.op(n) == Opcode::StoreValue));
    let SemanticOutcome::Return(Value::Uninit { bytes }) = run(&graph, &[]).unwrap() else {
        panic!("expected raw MaybeUninit snapshot");
    };
    assert_eq!(&*bytes, &[Some(1), None]);
}
