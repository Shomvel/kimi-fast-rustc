//! A source-domain obligation adds no executable representation demand.
use kfrc_body::*;
use kfrc_son::*;
use kfrc_x86::lower_function;

fn graph(requirements: usize) -> FunctionGraph {
    let mut types = RecordedTypes::new();
    let word = types.intern(
        TypeShape::Int {
            width: IntegerWidth::W64,
            signed: false,
        },
        Some(SizeAlign { size: 8, align: 8 }),
    );
    let boolean = types.intern(TypeShape::Bool, Some(SizeAlign { size: 1, align: 1 }));
    // Deliberately no representation answer: a proof-only enum must not ask.
    let enumeration = types.intern(
        TypeShape::Adt {
            name: "ProofOnly".into(),
        },
        None,
    );
    types.record_semantic_aggregate(
        enumeration,
        SemanticAggregate {
            fields: vec![],
            variants: vec![vec![word]; 128],
        },
    );
    types.record_semantic_enum(enumeration, (0..128).collect());
    let mut graph = FunctionGraph::new(
        "requirements_release",
        FunctionAbi {
            args: vec![PassMode::Direct],
            ret: PassMode::Direct,
            c_variadic: false,
        },
        types,
    );
    graph.level = GraphLevel::Specialized;
    let start = graph.add_node(
        Opcode::Start,
        NodeId::NONE,
        &[],
        0,
        ValueType::Tuple(vec![
            ValueType::Control,
            ValueType::Effect,
            ValueType::Rust(word),
        ]),
    );
    graph.start = start;
    let mut projection =
        |source, index, ty| graph.add_node(Opcode::Projection, NodeId::NONE, &[source], index, ty);
    let control = projection(start, 0, ValueType::Control);
    let mut effect = projection(start, 1, ValueType::Effect);
    let arg = projection(start, 2, ValueType::Rust(word));
    let zero = graph.add_constant(0, word);
    let condition = graph.add_node(
        Opcode::Compare,
        NodeId::NONE,
        &[arg, zero],
        ComparePredicate::Equal as u32,
        ValueType::Rust(boolean),
    );
    let branch = graph.add_node(
        Opcode::If,
        control,
        &[condition],
        0,
        ValueType::Tuple(vec![ValueType::Control; 2]),
    );
    let mut jumps = Vec::new();
    let mut alternatives = Vec::new();
    for (side, variant) in [0, 127].into_iter().enumerate() {
        let edge = graph.add_node(
            Opcode::Projection,
            NodeId::NONE,
            &[branch],
            side as u32,
            ValueType::Control,
        );
        jumps.push(graph.add_node(Opcode::Jump, edge, &[], 0, ValueType::Control));
        alternatives.push(graph.add_node(
            Opcode::MakeVariant,
            NodeId::NONE,
            &[arg],
            variant,
            ValueType::Rust(enumeration),
        ));
    }
    let mut control = graph.add_node(Opcode::Region, NodeId::NONE, &jumps, 0, ValueType::Control);
    let value = graph.add_node(
        Opcode::Phi,
        NodeId::NONE,
        &[control, alternatives[0], alternatives[1]],
        0,
        ValueType::Rust(enumeration),
    );
    for _ in 0..requirements {
        let requirement = graph.add_node(
            Opcode::RequireDefined,
            control,
            &[effect, value],
            0,
            ValueType::Tuple(vec![ValueType::Control, ValueType::Effect]),
        );
        control = graph.add_node(
            Opcode::Projection,
            NodeId::NONE,
            &[requirement],
            0,
            ValueType::Control,
        );
        effect = graph.add_node(
            Opcode::Projection,
            NodeId::NONE,
            &[requirement],
            1,
            ValueType::Effect,
        );
    }
    graph.add_node(
        Opcode::Return,
        control,
        &[effect, arg],
        0,
        ValueType::Control,
    );
    verify_graph(&mut graph).unwrap();
    graph
}

#[test]
fn redundant_requirements_add_no_branch_memory_move_or_layout_request() {
    let mut baseline = None;
    for requirements in [0, 1, 16] {
        let mut graph = graph(requirements);
        for value in [0, 1, 37] {
            assert_eq!(
                Interpreter::new(&graph)
                    .run_values(
                        &[Value::Scalar(value)],
                        &mut |_: &str, _: &[Value]| unreachable!()
                    )
                    .unwrap(),
                SemanticOutcome::Return(Value::Scalar(value))
            );
        }
        lower_layout(&mut graph).unwrap();
        verify_graph(&mut graph).unwrap();
        assert!(!graph.ids().any(|node| matches!(
            graph.op(node),
            Opcode::RequireDefined | Opcode::Load | Opcode::Store | Opcode::VariantWitness
        )));
        assert!(graph.stack_slots.is_empty());
        let machine = lower_function(&graph).unwrap();
        assert_eq!(
            &machine.instructions,
            baseline.get_or_insert_with(|| machine.instructions.clone())
        );
        for value in [0, 1, 37] {
            assert_eq!(
                kfrc_x86::machine::eval::evaluate(&machine, &[value]).unwrap(),
                value
            );
        }
    }
}

fn proof_only_read(requirement: bool) -> FunctionGraph {
    let mut graph = graph(0);
    let word = graph
        .rust_type(graph.find_projection(graph.start, 2).unwrap())
        .unwrap();
    let byte = graph.oracle.intern(
        TypeShape::Int {
            width: IntegerWidth::W8,
            signed: false,
        },
        Some(SizeAlign { size: 1, align: 1 }),
    );
    let enumeration = graph.oracle.intern(
        TypeShape::Adt {
            name: "ReadProofOnly".into(),
        },
        Some(SizeAlign { size: 16, align: 8 }),
    );
    graph.oracle.record_aggregate(
        enumeration,
        AggregateLayout {
            fields: vec![],
            variants: vec![
                vec![FieldLayout {
                    ty: word,
                    offset: 8
                }];
                2
            ],
        },
    );
    graph.oracle.record_semantic_enum(enumeration, vec![0, 1]);
    graph.oracle.enums.insert(
        enumeration,
        EnumLayout {
            tag: Some(FieldLayout {
                ty: byte,
                offset: 0,
            }),
            encoding: TagEncoding::Direct {
                discriminants: vec![0, 1],
            },
        },
    );
    let address_ty = graph.oracle.intern(
        TypeShape::PlaceAddress {
            pointee: enumeration,
        },
        None,
    );
    graph.data_objects.push(DataObject {
        symbol: "require_read_data".into(),
        bytes: [vec![1], vec![0; 7], 37u64.to_le_bytes().to_vec()].concat(),
        align: 8,
        ..Default::default()
    });
    graph.symbols.push(("require_read_data".into(), 0));
    let address = graph.add_node(
        Opcode::SymbolAddress,
        NodeId::NONE,
        &[],
        (graph.symbols.len() - 1) as u32,
        ValueType::Rust(address_ty),
    );
    let ret = graph
        .ids()
        .find(|&node| graph.op(node) == Opcode::Return)
        .unwrap();
    let load = graph.add_node(
        Opcode::LoadValue,
        graph.control(ret),
        &[graph.input(ret, 0), address],
        0,
        ValueType::Tuple(vec![
            ValueType::Control,
            ValueType::Effect,
            ValueType::Rust(enumeration),
        ]),
    );
    let mut control = graph.add_node(
        Opcode::Projection,
        NodeId::NONE,
        &[load],
        0,
        ValueType::Control,
    );
    let mut effect = graph.add_node(
        Opcode::Projection,
        NodeId::NONE,
        &[load],
        1,
        ValueType::Effect,
    );
    if requirement {
        let value = graph.add_node(
            Opcode::Projection,
            NodeId::NONE,
            &[load],
            2,
            ValueType::Rust(enumeration),
        );
        let req = graph.add_node(
            Opcode::RequireDefined,
            control,
            &[effect, value],
            0,
            ValueType::Tuple(vec![ValueType::Control, ValueType::Effect]),
        );
        control = graph.add_node(
            Opcode::Projection,
            NodeId::NONE,
            &[req],
            0,
            ValueType::Control,
        );
        effect = graph.add_node(
            Opcode::Projection,
            NodeId::NONE,
            &[req],
            1,
            ValueType::Effect,
        );
    }
    let callee = graph.add_callee(CallTarget {
        symbol: "trace_call".into(),
        abi: FunctionAbi {
            args: vec![],
            ret: PassMode::Direct,
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
            ValueType::Rust(word),
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
    graph.set_control(ret, control);
    graph.set_input(ret, 0, effect);
    verify_graph(&mut graph).unwrap();
    graph
}

#[test]
fn proof_only_snapshot_has_no_payload_loads_and_keeps_actual_calls() {
    let mut baseline = None;
    for requirement in [false, true] {
        let mut graph = proof_only_read(requirement);
        let mut trace = Vec::new();
        assert_eq!(
            Interpreter::new(&graph)
                .run_values(&[Value::Scalar(19)], &mut |name: &str, _: &[Value]| {
                    trace.push(name.to_owned());
                    Ok(Value::Scalar(0))
                })
                .unwrap(),
            SemanticOutcome::Return(Value::Scalar(19))
        );
        assert_eq!(trace, ["trace_call"]);
        lower_layout(&mut graph).unwrap();
        verify_graph(&mut graph).unwrap();
        assert!(!graph.ids().any(|node| matches!(
            graph.op(node),
            Opcode::Load | Opcode::Store | Opcode::RequireDefined
        )));
        trace.clear();
        assert_eq!(
            Interpreter::new(&graph)
                .run(&[19], &mut |name: &str, _: &[u64]| {
                    trace.push(name.to_owned());
                    Ok(0)
                })
                .unwrap(),
            Outcome::Return(19)
        );
        assert_eq!(trace, ["trace_call"]);
        let machine = lower_function(&graph).unwrap();
        assert_eq!(
            &machine.instructions,
            baseline.get_or_insert_with(|| machine.instructions.clone())
        );
    }
}
