//! Source ZST values are reconstructed only when publishing ignored transport.
use kfrc_body::*;
use kfrc_son::*;

fn fixture(call: bool) -> FunctionGraph {
    let mut types = RecordedTypes::new();
    let word = types.intern(
        TypeShape::Int {
            width: IntegerWidth::W64,
            signed: false,
        },
        Some(SizeAlign { size: 8, align: 8 }),
    );
    let never = types.intern(TypeShape::Never, Some(SizeAlign { size: 0, align: 1 }));
    let empty = types.intern(
        TypeShape::Array {
            element: never,
            length: 0,
        },
        Some(SizeAlign { size: 0, align: 1 }),
    );
    let enumeration = types.intern(
        TypeShape::Adt {
            name: "OnlySecondIsInhabited".into(),
        },
        Some(SizeAlign { size: 0, align: 1 }),
    );
    types.record_aggregate(
        enumeration,
        AggregateLayout {
            fields: vec![],
            variants: vec![
                vec![FieldLayout {
                    ty: never,
                    offset: 0,
                }],
                vec![FieldLayout {
                    ty: empty,
                    offset: 0,
                }],
            ],
        },
    );
    types.record_semantic_enum(enumeration, vec![0, 7]);
    types.record_semantic_inhabited_variants(enumeration, vec![1]);
    types.enums.insert(
        enumeration,
        EnumLayout {
            tag: None,
            encoding: TagEncoding::Single {
                variant: 1,
                discriminant: 7,
            },
        },
    );
    let abi = FunctionAbi {
        args: if call { vec![] } else { vec![PassMode::Ignore] },
        ret: PassMode::Direct,
        c_variadic: false,
    };
    let mut graph = FunctionGraph::new("ignored_zst", abi, types);
    let mut schema = vec![ValueType::Control, ValueType::Effect];
    if !call {
        schema.push(ValueType::Rust(enumeration));
    }
    let start = graph.add_node(
        Opcode::Start,
        NodeId::NONE,
        &[],
        0,
        ValueType::Tuple(schema),
    );
    graph.start = start;
    let mut control = graph.add_node(
        Opcode::Projection,
        NodeId::NONE,
        &[start],
        0,
        ValueType::Control,
    );
    let mut effect = graph.add_node(
        Opcode::Projection,
        NodeId::NONE,
        &[start],
        1,
        ValueType::Effect,
    );
    let source = if call {
        let callee = graph.add_callee(CallTarget {
            symbol: "make_zst".into(),
            abi: FunctionAbi {
                args: vec![],
                ret: PassMode::Ignore,
                c_variadic: false,
            },
        });
        let call = graph.add_node(
            Opcode::Call,
            control,
            &[effect],
            callee,
            ValueType::Tuple(vec![
                ValueType::Control,
                ValueType::Effect,
                ValueType::Rust(enumeration),
            ]),
        );
        control = graph.add_node(
            Opcode::Projection,
            NodeId::NONE,
            &[call],
            0,
            ValueType::Control,
        );
        effect = graph.add_node(
            Opcode::Projection,
            NodeId::NONE,
            &[call],
            1,
            ValueType::Effect,
        );
        call
    } else {
        start
    };
    let value = graph.add_node(
        Opcode::Projection,
        NodeId::NONE,
        &[source],
        2,
        ValueType::Rust(enumeration),
    );
    let result = graph.add_node(
        Opcode::Discriminant,
        NodeId::NONE,
        &[value],
        0,
        ValueType::Rust(word),
    );
    graph.add_node(
        Opcode::Return,
        control,
        &[effect, result],
        0,
        ValueType::Control,
    );
    verify_graph(&mut graph).unwrap();
    graph
}

#[test]
fn ignored_start_and_call_values_keep_the_only_inhabited_variant_and_effects() {
    for call in [false, true] {
        let mut graph = fixture(call);
        let value = Value::Variant {
            variant: 1,
            fields: vec![Value::Aggregate(vec![])],
        };
        let mut calls = 0;
        let arguments = if call { vec![] } else { vec![value.clone()] };
        assert_eq!(
            Interpreter::new(&graph)
                .run_values(&arguments, &mut |name: &str, _: &[Value]| {
                    assert_eq!(name, "make_zst");
                    calls += 1;
                    Ok(value.clone())
                })
                .unwrap(),
            SemanticOutcome::Return(Value::Scalar(7))
        );
        assert_eq!(calls, usize::from(call));
        lower_layout(&mut graph).unwrap();
        verify_graph(&mut graph).unwrap();
        assert!(
            !graph
                .ids()
                .any(|node| matches!(graph.op(node), Opcode::Load | Opcode::Store))
        );
        assert!(graph.stack_slots.is_empty());
        calls = 0;
        assert_eq!(
            Interpreter::new(&graph)
                .run(&[], &mut |name: &str, _: &[u64]| {
                    assert_eq!(name, "make_zst");
                    calls += 1;
                    Ok(0)
                })
                .unwrap(),
            Outcome::Return(7)
        );
        assert_eq!(calls, usize::from(call));
    }
}
