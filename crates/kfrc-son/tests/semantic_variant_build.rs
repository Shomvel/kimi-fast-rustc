//! Production Body construction retains logical enum control and scoped facts.
use kfrc_body::*;
use kfrc_son::{
    InterpretError, Interpreter, Opcode, SemanticOutcome, Value, ValueType, build_graph,
    verify_graph,
};

fn local(body: &mut BodyBuilder, ty: TyId) -> LocalId {
    body.add_local(LocalDecl {
        ty,
        mutability: Mutability::Mut,
        name: None,
    })
}
fn copy(local: LocalId) -> Operand {
    Operand::Copy(Place::local(local))
}
fn fixture() -> (BodyBuilder, LocalId, LocalId, LocalId, TyId) {
    let mut types = RecordedTypes::new();
    let word = types.intern(
        TypeShape::Int {
            width: IntegerWidth::W64,
            signed: false,
        },
        None,
    );
    let discriminant = types.intern(
        TypeShape::Int {
            width: IntegerWidth::W8,
            signed: true,
        },
        None,
    );
    let enumeration = types.intern(
        TypeShape::Adt {
            name: "SparseDiscriminants".into(),
        },
        None,
    );
    types.record_semantic_aggregate(
        enumeration,
        SemanticAggregate {
            fields: vec![],
            variants: vec![vec![word]; 3],
        },
    );
    types.record_semantic_enum(enumeration, vec![u128::MAX, 7, 42]);
    let mut body = BodyBuilder::new(
        "variant_switch",
        FunctionAbi {
            args: vec![PassMode::Direct],
            ret: PassMode::Direct,
            c_variadic: false,
        },
        types,
    );
    let ret = local(&mut body, word);
    let value = local(&mut body, enumeration);
    let tag = local(&mut body, discriminant);
    body.set_arg_count(1);
    body.push_statement(StatementKind::Assign(
        Place::local(tag),
        Rvalue::Discriminant(Place::local(value)),
    ));
    (body, ret, value, tag, word)
}
fn payload(body: &mut BodyBuilder, value: LocalId, variant: u32, word: TyId) -> Operand {
    let projection = body.add_projection(vec![
        ProjectionElem::Downcast(VariantIndex(variant)),
        ProjectionElem::Field(FieldIndex(0), word),
    ]);
    Operand::Copy(Place {
        local: value,
        projection,
    })
}
fn run(
    graph: &kfrc_son::FunctionGraph,
    variant: u32,
    value: u64,
) -> Result<SemanticOutcome, InterpretError> {
    Interpreter::new(graph).run_values(
        &[Value::Variant {
            variant,
            fields: vec![Value::Scalar(value)],
        }],
        &mut |name: &str, _: &[Value]| Err(InterpretError::UnknownCallee(name.into())),
    )
}

#[test]
fn sparse_and_negative_discriminants_build_one_variant_switch_and_proven_fields() {
    let (mut body, ret, value, tag, word) = fixture();
    body.finish_block(
        Terminator::SwitchInt {
            discriminant: copy(tag),
            targets: SwitchTargets {
                cases: vec![(255, BlockId(1)), (7, BlockId(2))],
                otherwise: BlockId(3),
            },
        },
        false,
    );
    for variant in 0..3 {
        let field = payload(&mut body, value, variant, word);
        body.push_statement(StatementKind::Assign(Place::local(ret), Rvalue::Use(field)));
        body.finish_block(Terminator::Return, false);
    }
    let mut graph = build_graph(&body.finish()).unwrap();
    verify_graph(&mut graph).unwrap();
    assert_eq!(
        graph
            .ids()
            .filter(|&n| graph.op(n) == Opcode::VariantSwitch)
            .count(),
        1
    );
    assert!(!graph.ids().any(|n| matches!(
        graph.op(n),
        Opcode::If | Opcode::Compare | Opcode::Discriminant
    )));
    assert_eq!(
        graph
            .ids()
            .filter(|&n| graph.op(n) == Opcode::VariantWitness)
            .count(),
        3
    );
    for node in graph.ids().filter(|&n| graph.op(n) == Opcode::VariantField) {
        let source = graph.input(node, 0);
        let witness = graph.input(node, 1);
        assert_eq!(graph.op(witness), Opcode::VariantWitness);
        assert_eq!(graph.value_type(witness), &ValueType::Witness);
        assert_eq!(graph.input(witness, 0), source);
        assert_eq!(graph.op(source), Opcode::Projection);
        assert_eq!(graph.input(source, 0), graph.start);
        assert_eq!(
            graph.payload(witness),
            graph.variant_fields[graph.payload(node) as usize].0
        );
    }
    for variant in 0..3 {
        assert_eq!(
            run(&graph, variant, 19 + u64::from(variant)),
            Ok(SemanticOutcome::Return(Value::Scalar(
                19 + u64::from(variant)
            )))
        );
    }
}

#[test]
fn default_with_multiple_variants_does_not_invent_a_unique_variant_fact() {
    let (mut body, ret, value, tag, word) = fixture();
    body.finish_block(
        Terminator::SwitchInt {
            discriminant: copy(tag),
            targets: SwitchTargets {
                cases: vec![(255, BlockId(1))],
                otherwise: BlockId(2),
            },
        },
        false,
    );
    for variant in [0, 1] {
        let field = payload(&mut body, value, variant, word);
        body.push_statement(StatementKind::Assign(Place::local(ret), Rvalue::Use(field)));
        body.finish_block(Terminator::Return, false);
    }
    let mut graph = build_graph(&body.finish()).unwrap();
    verify_graph(&mut graph).unwrap();
    assert_eq!(
        graph
            .ids()
            .filter(|&n| graph.op(n) == Opcode::VariantWitness)
            .count(),
        1
    );
    assert_eq!(
        run(&graph, 1, 77),
        Ok(SemanticOutcome::Return(Value::Scalar(77)))
    );
    assert!(matches!(
        run(&graph, 2, 88),
        Err(InterpretError::PoisonObserved(_))
    ));
}

#[test]
fn merging_distinct_variant_edges_never_licenses_a_witness() {
    let (mut body, ret, value, tag, word) = fixture();
    body.finish_block(
        Terminator::SwitchInt {
            discriminant: copy(tag),
            targets: SwitchTargets {
                cases: vec![(255, BlockId(1)), (7, BlockId(1))],
                otherwise: BlockId(2),
            },
        },
        false,
    );
    for variant in [0, 2] {
        let field = payload(&mut body, value, variant, word);
        body.push_statement(StatementKind::Assign(Place::local(ret), Rvalue::Use(field)));
        body.finish_block(Terminator::Return, false);
    }
    let mut graph = build_graph(&body.finish()).unwrap();
    verify_graph(&mut graph).unwrap();
    assert_eq!(
        graph
            .ids()
            .filter(|&n| graph.op(n) == Opcode::VariantWitness)
            .count(),
        1
    );
    assert_eq!(
        run(&graph, 0, 17),
        Ok(SemanticOutcome::Return(Value::Scalar(17)))
    );
    assert!(matches!(
        run(&graph, 1, 18),
        Err(InterpretError::PoisonObserved(_))
    ));
}

#[test]
fn downcast_without_switch_does_not_add_an_assumption_or_observe_dead_poison() {
    let (mut body, ret, value, _, word) = fixture();
    let temporary = local(&mut body, word);
    let field = payload(&mut body, value, 0, word);
    body.push_statement(StatementKind::Assign(
        Place::local(temporary),
        Rvalue::Use(field),
    ));
    let answer = body.add_constant(Constant {
        ty: word,
        kind: ConstantKind::Scalar(91),
    });
    body.push_statement(StatementKind::Assign(
        Place::local(ret),
        Rvalue::Use(Operand::Constant(answer)),
    ));
    body.finish_block(Terminator::Return, false);
    let mut graph = build_graph(&body.finish()).unwrap();
    verify_graph(&mut graph).unwrap();
    assert!(!graph.ids().any(|n| graph.op(n) == Opcode::VariantWitness));
    assert_eq!(
        run(&graph, 2, 18),
        Ok(SemanticOutcome::Return(Value::Scalar(91)))
    );
}

#[test]
fn sibling_fields_share_one_witness_without_copying_the_enum_value() {
    let mut types = RecordedTypes::new();
    let word = types.intern(
        TypeShape::Int {
            width: IntegerWidth::W64,
            signed: false,
        },
        None,
    );
    let pair = types.intern(TypeShape::Tuple(vec![word, word]), None);
    let enumeration = types.intern(
        TypeShape::Adt {
            name: "PairChoice".into(),
        },
        None,
    );
    types.record_semantic_aggregate(
        enumeration,
        SemanticAggregate {
            fields: vec![],
            variants: vec![vec![word, word], vec![word, word]],
        },
    );
    types.record_semantic_enum(enumeration, vec![0, 1]);
    let mut b = BodyBuilder::new(
        "shared_witness",
        FunctionAbi {
            args: vec![PassMode::Direct],
            ret: PassMode::Direct,
            c_variadic: false,
        },
        types,
    );
    let ret = local(&mut b, pair);
    let source = local(&mut b, enumeration);
    let tag = local(&mut b, word);
    b.set_arg_count(1);
    b.push_statement(StatementKind::Assign(
        Place::local(tag),
        Rvalue::Discriminant(Place::local(source)),
    ));
    b.finish_block(
        Terminator::SwitchInt {
            discriminant: copy(tag),
            targets: SwitchTargets {
                cases: vec![(0, BlockId(1))],
                otherwise: BlockId(2),
            },
        },
        false,
    );
    for variant in 0..2 {
        let fields = (0..2)
            .map(|field| {
                let projection = b.add_projection(vec![
                    ProjectionElem::Downcast(VariantIndex(variant)),
                    ProjectionElem::Field(FieldIndex(field), word),
                ]);
                Operand::Copy(Place {
                    local: source,
                    projection,
                })
            })
            .collect();
        b.push_statement(StatementKind::Assign(
            Place::local(ret),
            Rvalue::Aggregate(AggregateKind::Tuple, fields),
        ));
        b.finish_block(Terminator::Return, false);
    }
    let mut graph = build_graph(&b.finish()).unwrap();
    verify_graph(&mut graph).unwrap();
    assert_eq!(
        graph
            .ids()
            .filter(|&n| graph.op(n) == Opcode::VariantWitness)
            .count(),
        2
    );
    for variant in 0..2 {
        let fields: Vec<_> = graph
            .ids()
            .filter(|&n| {
                graph.op(n) == Opcode::VariantField
                    && graph.variant_fields[graph.payload(n) as usize].0 == variant
            })
            .collect();
        assert_eq!(fields.len(), 2);
        assert_eq!(graph.input(fields[0], 0), graph.input(fields[1], 0));
        assert_eq!(graph.input(fields[0], 1), graph.input(fields[1], 1));
        let result = Interpreter::new(&graph)
            .run_values(
                &[Value::Variant {
                    variant,
                    fields: vec![Value::Scalar(7), Value::Scalar(11)],
                }],
                &mut |_: &str, _: &[Value]| unreachable!(),
            )
            .unwrap();
        assert_eq!(
            result,
            SemanticOutcome::Return(Value::Aggregate(vec![Value::Scalar(7), Value::Scalar(11)]))
        );
    }
}

#[test]
fn parent_variant_witness_does_not_certify_an_unchecked_nested_variant() {
    let mut types = RecordedTypes::new();
    let word = types.intern(
        TypeShape::Int {
            width: IntegerWidth::W64,
            signed: false,
        },
        None,
    );
    let child = types.intern(
        TypeShape::Adt {
            name: "NestedChoice".into(),
        },
        None,
    );
    types.record_semantic_aggregate(
        child,
        SemanticAggregate {
            fields: vec![],
            variants: vec![vec![word], vec![word]],
        },
    );
    types.record_semantic_enum(child, vec![0, 1]);
    let outer = types.intern(
        TypeShape::Adt {
            name: "OuterChoice".into(),
        },
        None,
    );
    types.record_semantic_aggregate(
        outer,
        SemanticAggregate {
            fields: vec![],
            variants: vec![vec![child], vec![child]],
        },
    );
    types.record_semantic_enum(outer, vec![0, 1]);
    let mut b = BodyBuilder::new(
        "nested_witness",
        FunctionAbi {
            args: vec![PassMode::Direct],
            ret: PassMode::Direct,
            c_variadic: false,
        },
        types,
    );
    let ret = local(&mut b, word);
    let parent = local(&mut b, outer);
    let tag = local(&mut b, word);
    b.set_arg_count(1);
    b.push_statement(StatementKind::Assign(
        Place::local(tag),
        Rvalue::Discriminant(Place::local(parent)),
    ));
    b.finish_block(
        Terminator::SwitchInt {
            discriminant: copy(tag),
            targets: SwitchTargets {
                cases: vec![(0, BlockId(1))],
                otherwise: BlockId(2),
            },
        },
        false,
    );
    let nested = b.add_projection(vec![
        ProjectionElem::Downcast(VariantIndex(0)),
        ProjectionElem::Field(FieldIndex(0), child),
        ProjectionElem::Downcast(VariantIndex(0)),
        ProjectionElem::Field(FieldIndex(0), word),
    ]);
    b.push_statement(StatementKind::Assign(
        Place::local(ret),
        Rvalue::Use(Operand::Copy(Place {
            local: parent,
            projection: nested,
        })),
    ));
    b.finish_block(Terminator::Return, false);
    let zero = b.add_constant(Constant {
        ty: word,
        kind: ConstantKind::Scalar(0),
    });
    b.push_statement(StatementKind::Assign(
        Place::local(ret),
        Rvalue::Use(Operand::Constant(zero)),
    ));
    b.finish_block(Terminator::Return, false);
    let mut graph = build_graph(&b.finish()).unwrap();
    verify_graph(&mut graph).unwrap();
    for field in graph.ids().filter(|&n| graph.op(n) == Opcode::VariantField) {
        let source = graph.input(field, 0);
        assert_eq!(
            graph.inputs(field).len(),
            if graph.rust_type(source) == Some(outer) {
                2
            } else {
                1
            }
        );
    }
    let outcome = Interpreter::new(&graph).run_values(
        &[Value::Variant {
            variant: 0,
            fields: vec![Value::Variant {
                variant: 1,
                fields: vec![Value::Scalar(9)],
            }],
        }],
        &mut |_: &str, _: &[Value]| unreachable!(),
    );
    assert!(matches!(outcome, Err(InterpretError::PoisonObserved(_))));
}
