//! The Body builder synthesizes Bool for MIR's three-way comparison even when
//! the source type table contains no Bool. Its facts must match rustc's Bool.
use kfrc_body::*;
use kfrc_son::{
    CallTarget, FunctionGraph, GraphLevel, InlineBudget, InlineSkip, InterpretError, Interpreter,
    NodeId, Opcode, SemanticOutcome, Value, ValueType, build_graph, inline_calls, verify_graph,
};
use std::collections::HashMap;

fn abi() -> FunctionAbi {
    FunctionAbi {
        args: vec![PassMode::Direct; 2],
        ret: PassMode::Direct,
        c_variadic: false,
    }
}

fn types() -> (RecordedTypes, TyId, TyId) {
    let mut types = RecordedTypes::new();
    let word = types.intern(
        TypeShape::Int {
            width: IntegerWidth::Pointer,
            signed: false,
        },
        Some(SizeAlign { size: 8, align: 8 }),
    );
    let tag = types.intern(
        TypeShape::Int {
            width: IntegerWidth::W8,
            signed: true,
        },
        Some(SizeAlign { size: 1, align: 1 }),
    );
    let ordering = types.intern(
        TypeShape::Adt {
            name: "Ordering#producer-identity".into(),
        },
        Some(SizeAlign { size: 1, align: 1 }),
    );
    types.record_aggregate(
        ordering,
        AggregateLayout {
            fields: vec![],
            variants: vec![vec![], vec![], vec![]],
        },
    );
    types.record_semantic_enum(ordering, vec![u128::MAX, 0, 1]);
    types.record_ordering(ordering);
    types.enums.insert(
        ordering,
        EnumLayout {
            tag: Some(FieldLayout { ty: tag, offset: 0 }),
            encoding: TagEncoding::Direct {
                discriminants: vec![u128::MAX, 0, 1],
            },
        },
    );
    (types, word, ordering)
}

fn cmp_callee() -> FunctionGraph {
    let (types, word, ordering) = types();
    let mut body = BodyBuilder::new("ord_cmp", abi(), types);
    let result = body.add_local(LocalDecl {
        ty: ordering,
        mutability: Mutability::Mut,
        name: None,
    });
    let left = body.add_local(LocalDecl {
        ty: word,
        mutability: Mutability::Not,
        name: None,
    });
    let right = body.add_local(LocalDecl {
        ty: word,
        mutability: Mutability::Not,
        name: None,
    });
    body.set_arg_count(2);
    body.push_statement(StatementKind::Assign(
        Place::local(result),
        Rvalue::BinaryOp(
            BinaryOp::Cmp,
            Operand::Copy(Place::local(left)),
            Operand::Copy(Place::local(right)),
        ),
    ));
    body.finish_block(Terminator::Return, false);
    let mut graph = build_graph(&body.finish()).unwrap();
    verify_graph(&mut graph).unwrap();
    graph
}

fn caller(validity: ScalarValidRange) -> FunctionGraph {
    let (mut types, word, ordering) = types();
    let boolean = types.intern(TypeShape::Bool, Some(SizeAlign { size: 1, align: 1 }));
    types.record_scalar_validity(boolean, ScalarValidity::Scalar(validity));
    let mut graph = FunctionGraph::new("caller", abi(), types);
    graph.level = GraphLevel::Specialized;
    graph.start = graph.add_node(
        Opcode::Start,
        NodeId::NONE,
        &[],
        0,
        ValueType::Tuple(vec![
            ValueType::Control,
            ValueType::Effect,
            ValueType::Rust(word),
            ValueType::Rust(word),
        ]),
    );
    let control = graph.add_node(
        Opcode::Projection,
        NodeId::NONE,
        &[graph.start],
        0,
        ValueType::Control,
    );
    let effect = graph.add_node(
        Opcode::Projection,
        NodeId::NONE,
        &[graph.start],
        1,
        ValueType::Effect,
    );
    let left = graph.add_node(
        Opcode::Projection,
        NodeId::NONE,
        &[graph.start],
        2,
        ValueType::Rust(word),
    );
    let right = graph.add_node(
        Opcode::Projection,
        NodeId::NONE,
        &[graph.start],
        3,
        ValueType::Rust(word),
    );
    let callee = graph.add_callee(CallTarget {
        symbol: "ord_cmp".into(),
        abi: abi(),
    });
    let call = graph.add_node(
        Opcode::Call,
        control,
        &[effect, left, right],
        callee,
        ValueType::Tuple(vec![
            ValueType::Control,
            ValueType::Effect,
            ValueType::Rust(ordering),
        ]),
    );
    let control = graph.add_node(
        Opcode::Projection,
        NodeId::NONE,
        &[call],
        0,
        ValueType::Control,
    );
    let effect = graph.add_node(
        Opcode::Projection,
        NodeId::NONE,
        &[call],
        1,
        ValueType::Effect,
    );
    let value = graph.add_node(
        Opcode::Projection,
        NodeId::NONE,
        &[call],
        2,
        ValueType::Rust(ordering),
    );
    graph.add_node(
        Opcode::Return,
        control,
        &[effect, value],
        0,
        ValueType::Control,
    );
    verify_graph(&mut graph).unwrap();
    graph
}

#[test]
fn body_cmp_synthesizes_bool_facts_compatible_with_a_rustc_style_caller() {
    let callee = cmp_callee();
    let boolean = callee
        .ids()
        .find_map(|node| {
            (callee.op(node) == Opcode::Compare).then(|| callee.rust_type(node).unwrap())
        })
        .unwrap();
    let range = ScalarValidRange::new(8, 0, 1).unwrap();
    assert_eq!(callee.oracle.scalar_valid_range(boolean), Some(range));
    let mut caller = caller(range);
    let provider = HashMap::from([(callee.symbol.clone(), callee)]);
    let stats = inline_calls(&mut caller, &provider, InlineBudget::default()).unwrap();
    assert_eq!(stats.calls_inlined, 1, "{:?}", stats.skipped);
    assert!(!caller.ids().any(|node| caller.op(node) == Opcode::Call));
    verify_graph(&mut caller).unwrap();
    for left in [0, 1, u64::MAX] {
        for right in [0, 1, u64::MAX] {
            let expected = match left.cmp(&right) {
                std::cmp::Ordering::Less => 0,
                std::cmp::Ordering::Equal => 1,
                std::cmp::Ordering::Greater => 2,
            };
            let output = Interpreter::new(&caller)
                .run_values(
                    &[Value::Scalar(left), Value::Scalar(right)],
                    &mut |symbol: &str, _: &[Value]| {
                        Err(InterpretError::UnknownCallee(symbol.into()))
                    },
                )
                .unwrap();
            assert_eq!(
                output,
                SemanticOutcome::Return(Value::Variant {
                    variant: expected,
                    fields: vec![]
                })
            );
        }
    }
}

#[test]
fn a_real_bool_range_conflict_still_rejects_inlining() {
    let callee = cmp_callee();
    let mut caller = caller(ScalarValidRange::new(8, 0, 0).unwrap());
    let provider = HashMap::from([(callee.symbol.clone(), callee)]);
    let stats = inline_calls(&mut caller, &provider, InlineBudget::default()).unwrap();
    assert_eq!(stats.calls_inlined, 0);
    assert_eq!(
        stats.skipped,
        vec![("ord_cmp".into(), InlineSkip::TypeConflict)]
    );
}
