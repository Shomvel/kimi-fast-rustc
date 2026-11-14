//! Leaf demand follows Rust aggregate SSA without weakening snapshot semantics.
use kfrc_body::{
    AggregateLayout, EnumLayout, FieldLayout, FunctionAbi, IntegerWidth, PassMode, RecordedTypes,
    SizeAlign, TagEncoding, TypeShape,
};
use kfrc_son::{
    ComparePredicate, FunctionGraph, GraphLevel, InterpretError, Interpreter, NodeId, Opcode,
    Outcome, SemanticOutcome, Value, ValueType, lower_layout, verify_graph,
};

fn fixture(enumeration: bool, tag_only: bool) -> FunctionGraph {
    fixture_fields(enumeration, tag_only, 8)
}

fn fixture_fields(enumeration: bool, tag_only: bool, count: usize) -> FunctionGraph {
    let mut types = RecordedTypes::new();
    let word = types.intern(
        TypeShape::Int {
            width: IntegerWidth::W64,
            signed: false,
        },
        Some(SizeAlign { size: 8, align: 8 }),
    );
    let boolean = types.intern(TypeShape::Bool, Some(SizeAlign { size: 1, align: 1 }));
    let aggregate = types.intern(
        if enumeration {
            TypeShape::Adt {
                name: "DemandEnum".into(),
            }
        } else {
            TypeShape::Tuple(vec![word; count])
        },
        Some(SizeAlign {
            size: 8 * (count as u64 + u64::from(enumeration)),
            align: 8,
        }),
    );
    let fields: Vec<_> = (0..count)
        .map(|i| FieldLayout {
            ty: word,
            offset: 8 * (i as u64 + u64::from(enumeration)),
        })
        .collect();
    types.record_aggregate(
        aggregate,
        AggregateLayout {
            fields: if enumeration { vec![] } else { fields.clone() },
            variants: if enumeration {
                vec![fields.clone(), fields]
            } else {
                vec![]
            },
        },
    );
    if enumeration {
        types.record_semantic_enum(aggregate, vec![7, 19]);
        types.enums.insert(
            aggregate,
            EnumLayout {
                tag: Some(FieldLayout {
                    ty: word,
                    offset: 0,
                }),
                encoding: TagEncoding::Direct {
                    discriminants: vec![7, 19],
                },
            },
        );
    }
    let mut g = FunctionGraph::new(
        "leaf_demand",
        FunctionAbi {
            args: vec![PassMode::Direct],
            ret: PassMode::Direct,
            c_variadic: false,
        },
        types,
    );
    g.level = GraphLevel::Specialized;
    let start = g.add_node(
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
    g.start = start;
    let control = g.add_node(
        Opcode::Projection,
        NodeId::NONE,
        &[start],
        0,
        ValueType::Control,
    );
    let effect = g.add_node(
        Opcode::Projection,
        NodeId::NONE,
        &[start],
        1,
        ValueType::Effect,
    );
    let input = g.add_node(
        Opcode::Projection,
        NodeId::NONE,
        &[start],
        2,
        ValueType::Rust(word),
    );
    let zero = g.add_constant(0, word);
    let condition = g.add_node(
        Opcode::Compare,
        NodeId::NONE,
        &[input, zero],
        ComparePredicate::Equal as u32,
        ValueType::Rust(boolean),
    );
    let branch = g.add_node(
        Opcode::If,
        control,
        &[condition],
        0,
        ValueType::Tuple(vec![ValueType::Control, ValueType::Control]),
    );
    let edges = [0, 1].map(|i| {
        let edge = g.add_node(
            Opcode::Projection,
            NodeId::NONE,
            &[branch],
            i,
            ValueType::Control,
        );
        g.add_node(Opcode::Jump, edge, &[], 0, ValueType::Control)
    });
    let region = g.add_node(Opcode::Region, NodeId::NONE, &edges, 0, ValueType::Control);
    let values = [0, 1].map(|arm| {
        let fields: Vec<_> = (0..count)
            .map(|i| g.add_constant((100 * arm as usize + i) as u128, word))
            .collect();
        g.add_node(
            if enumeration {
                Opcode::MakeVariant
            } else {
                Opcode::Aggregate
            },
            NodeId::NONE,
            &fields,
            if enumeration { arm } else { 0 },
            ValueType::Rust(aggregate),
        )
    });
    let phi = g.add_node(
        Opcode::Phi,
        NodeId::NONE,
        &[region, values[0], values[1]],
        0,
        ValueType::Rust(aggregate),
    );
    let result = if tag_only {
        g.add_node(
            Opcode::Discriminant,
            NodeId::NONE,
            &[phi],
            0,
            ValueType::Rust(word),
        )
    } else {
        g.add_node(
            Opcode::Field,
            NodeId::NONE,
            &[phi],
            3,
            ValueType::Rust(word),
        )
    };
    g.add_node(
        Opcode::Return,
        region,
        &[effect, result],
        0,
        ValueType::Control,
    );
    verify_graph(&mut g).unwrap();
    g
}

fn compare(mut graph: FunctionGraph, expected: [u64; 2], expected_phis: usize) {
    for (input, &result) in expected.iter().enumerate() {
        assert_eq!(
            Interpreter::new(&graph)
                .run_values(
                    &[Value::Scalar(input as u64)],
                    &mut |name: &str, _: &[Value]| Err(InterpretError::UnknownCallee(name.into()))
                )
                .unwrap(),
            SemanticOutcome::Return(Value::Scalar(result))
        );
    }
    let stats = lower_layout(&mut graph).unwrap();
    verify_graph(&mut graph).unwrap();
    assert_eq!(stats.scalar_phis, expected_phis, "{}", graph.dump());
    assert_eq!(
        graph
            .ids()
            .filter(|&n| graph.op(n) == Opcode::Undefined)
            .count(),
        0
    );
    for (input, &result) in expected.iter().enumerate() {
        assert_eq!(
            Interpreter::new(&graph)
                .run(&[input as u64], &mut |name: &str, _: &[u64]| Err(
                    InterpretError::UnknownCallee(name.into())
                ))
                .unwrap(),
            Outcome::Return(result)
        );
    }
}

#[test]
fn one_used_field_creates_one_phi_from_eight_fields() {
    compare(fixture(false, false), [3, 103], 1);
}
#[test]
fn tag_only_enum_phi_never_materializes_inactive_payloads() {
    compare(fixture(true, true), [7, 19], 1);
}

#[test]
fn sparse_snapshot_reads_at_the_original_effect_before_alias_overwrite() {
    use kfrc_body::Mutability;
    let mut g = fixture(false, false);
    let ret = g.ids().find(|&n| g.op(n) == Opcode::Return).unwrap();
    let old_field = g.input(ret, 1);
    let aggregate = g.input(old_field, 0);
    let replacement = g.input(aggregate, 1);
    let ty = g.rust_type(aggregate).unwrap();
    let word = g.rust_type(old_field).unwrap();
    let mut control = g.control(ret);
    let mut effect = g.input(ret, 0);
    g.remove_node(ret);
    g.remove_node(old_field);
    let pointer = g.oracle.intern(
        TypeShape::RawPointer {
            mutability: Mutability::Mut,
            pointee: ty,
        },
        Some(SizeAlign { size: 8, align: 8 }),
    );
    let slot = g.add_node(
        Opcode::LocalSlot,
        NodeId::NONE,
        &[],
        0,
        ValueType::Rust(pointer),
    );
    let mut fixed = |g: &mut FunctionGraph, op, inputs: &[NodeId], result| {
        let mut types = vec![ValueType::Control, ValueType::Effect];
        if let Some(ty) = result {
            types.push(ValueType::Rust(ty));
        }
        let mut args = vec![effect];
        args.extend_from_slice(inputs);
        let node = g.add_node(op, control, &args, 0, ValueType::Tuple(types));
        control = g.add_node(
            Opcode::Projection,
            NodeId::NONE,
            &[node],
            0,
            ValueType::Control,
        );
        effect = g.add_node(
            Opcode::Projection,
            NodeId::NONE,
            &[node],
            1,
            ValueType::Effect,
        );
        result.map(|ty| {
            g.add_node(
                Opcode::Projection,
                NodeId::NONE,
                &[node],
                2,
                ValueType::Rust(ty),
            )
        })
    };
    fixed(&mut g, Opcode::StoreValue, &[slot, aggregate], None);
    let snapshot = fixed(&mut g, Opcode::LoadValue, &[slot], Some(ty)).unwrap();
    fixed(&mut g, Opcode::StoreValue, &[slot, replacement], None);
    let field = g.add_node(
        Opcode::Field,
        NodeId::NONE,
        &[snapshot],
        3,
        ValueType::Rust(word),
    );
    g.add_node(
        Opcode::Return,
        control,
        &[effect, field],
        0,
        ValueType::Control,
    );
    verify_graph(&mut g).unwrap();
    let stats = lower_layout(&mut g).unwrap();
    verify_graph(&mut g).unwrap();
    assert_eq!(stats.field_loads, 1, "{}", g.dump());
    for (input, result) in [3, 103].into_iter().enumerate() {
        assert_eq!(
            Interpreter::new(&g)
                .run(&[input as u64], &mut |name: &str, _: &[u64]| Err(
                    InterpretError::UnknownCallee(name.into())
                ))
                .unwrap(),
            Outcome::Return(result)
        );
    }
}

fn transport_fixture() -> FunctionGraph {
    use kfrc_body::Mutability;
    let mut g = fixture(true, true);
    let ret = g.ids().find(|&n| g.op(n) == Opcode::Return).unwrap();
    let old_tag = g.input(ret, 1);
    let value = g.input(old_tag, 0);
    let ty = g.rust_type(value).unwrap();
    let word = g.rust_type(old_tag).unwrap();
    let control = g.control(ret);
    let effect = g.input(ret, 0);
    g.remove_node(ret);
    g.remove_node(old_tag);
    let pointer = g.oracle.intern(
        TypeShape::RawPointer {
            mutability: Mutability::Mut,
            pointee: ty,
        },
        Some(SizeAlign { size: 8, align: 8 }),
    );
    let address = g.add_node(
        Opcode::LocalSlot,
        NodeId::NONE,
        &[],
        0,
        ValueType::Rust(pointer),
    );
    let store = g.add_node(
        Opcode::StoreValue,
        control,
        &[effect, address, value],
        0,
        ValueType::Tuple(vec![ValueType::Control, ValueType::Effect]),
    );
    let control = g.add_node(
        Opcode::Projection,
        NodeId::NONE,
        &[store],
        0,
        ValueType::Control,
    );
    let effect = g.add_node(
        Opcode::Projection,
        NodeId::NONE,
        &[store],
        1,
        ValueType::Effect,
    );
    let load = g.add_node(
        Opcode::LoadValue,
        control,
        &[effect, address],
        0,
        ValueType::Tuple(vec![
            ValueType::Control,
            ValueType::Effect,
            ValueType::Rust(ty),
        ]),
    );
    let control = g.add_node(
        Opcode::Projection,
        NodeId::NONE,
        &[load],
        0,
        ValueType::Control,
    );
    let effect = g.add_node(
        Opcode::Projection,
        NodeId::NONE,
        &[load],
        1,
        ValueType::Effect,
    );
    let snapshot = g.add_node(
        Opcode::Projection,
        NodeId::NONE,
        &[load],
        2,
        ValueType::Rust(ty),
    );
    let tag = g.add_node(
        Opcode::Discriminant,
        NodeId::NONE,
        &[snapshot],
        0,
        ValueType::Rust(word),
    );
    g.add_node(
        Opcode::Return,
        control,
        &[effect, tag],
        0,
        ValueType::Control,
    );
    verify_graph(&mut g).unwrap();
    g
}

#[test]
fn whole_enum_phi_uses_owned_transport_and_keeps_discriminant() {
    let mut graph = transport_fixture();
    compare(graph.clone(), [7, 19], 0);
    let stats = lower_layout(&mut graph).unwrap();
    assert_eq!(stats.owned_transport_phis, 1);
    assert_eq!(
        graph
            .ids()
            .filter(|&n| graph.op(n) == Opcode::Call
                && graph.callees[graph.payload(n) as usize].symbol == "memmove")
            .count(),
        1,
        "only the final store transports bytes"
    );
    assert_eq!(
        graph
            .ids()
            .filter(|&n| graph.op(n) == Opcode::Phi
                && matches!(graph.value_type(n), ValueType::Rust(_)))
            .count(),
        1,
        "one immutable pointer Phi"
    );
}

#[test]
fn inner_enum_phi_without_data_backedge_is_not_immutable_in_an_outer_loop() {
    let mut g = transport_fixture();
    let branch = g.ids().find(|&n| g.op(n) == Opcode::If).unwrap();
    let ret = g.ids().find(|&n| g.op(n) == Opcode::Return).unwrap();
    let store = g.ids().find(|&n| g.op(n) == Opcode::StoreValue).unwrap();
    let old_control = g.control(branch);
    let old_effect = g.input(store, 0);
    let result = g.input(ret, 1);
    let word = g.rust_type(result).unwrap();
    let boolean = g.rust_type(g.input(branch, 0)).unwrap();
    let entry = g.add_node(Opcode::Jump, old_control, &[], 0, ValueType::Control);
    let back = g.add_node(Opcode::Jump, NodeId::NONE, &[], 0, ValueType::Control);
    let loop_region = g.add_node(
        Opcode::Region,
        NodeId::NONE,
        &[entry, back],
        0,
        ValueType::Control,
    );
    let zero = g.add_constant(0, word);
    let iteration = g.add_node(
        Opcode::Phi,
        NodeId::NONE,
        &[loop_region, zero, zero],
        0,
        ValueType::Rust(word),
    );
    let one = g.add_constant(1, word);
    let next = g.add_node(
        Opcode::Add,
        NodeId::NONE,
        &[iteration, one],
        0,
        ValueType::Rust(word),
    );
    g.set_input(iteration, 2, next);
    let effect = g.add_node(
        Opcode::Phi,
        NodeId::NONE,
        &[loop_region, old_effect, g.input(ret, 0)],
        0,
        ValueType::Effect,
    );
    g.set_control(branch, loop_region);
    g.set_input(store, 0, effect);
    let three = g.add_constant(3, word);
    let again = g.add_node(
        Opcode::Compare,
        NodeId::NONE,
        &[next, three],
        ComparePredicate::UnsignedLess as u32,
        ValueType::Rust(boolean),
    );
    let repeat = g.add_node(
        Opcode::If,
        g.control(ret),
        &[again],
        0,
        ValueType::Tuple(vec![ValueType::Control, ValueType::Control]),
    );
    let yes = g.add_node(
        Opcode::Projection,
        NodeId::NONE,
        &[repeat],
        0,
        ValueType::Control,
    );
    let no = g.add_node(
        Opcode::Projection,
        NodeId::NONE,
        &[repeat],
        1,
        ValueType::Control,
    );
    g.set_control(back, yes);
    g.set_control(ret, no);
    verify_graph(&mut g).unwrap();
    for (input, expected) in [7, 19].into_iter().enumerate() {
        assert_eq!(
            Interpreter::new(&g)
                .run_values(
                    &[Value::Scalar(input as u64)],
                    &mut |name: &str, _: &[Value]| Err(InterpretError::UnknownCallee(name.into()))
                )
                .unwrap(),
            SemanticOutcome::Return(Value::Scalar(expected))
        );
    }
    let stats = lower_layout(&mut g).unwrap();
    verify_graph(&mut g).unwrap();
    assert_eq!(stats.owned_transport_phis, 0);
    for (input, expected) in [7, 19].into_iter().enumerate() {
        assert_eq!(
            Interpreter::new(&g)
                .run(&[input as u64], &mut |name: &str, _: &[u64]| Err(
                    InterpretError::UnknownCallee(name.into())
                ))
                .unwrap(),
            Outcome::Return(expected)
        );
    }
}

#[test]
fn immutable_enum_return_reuses_snapshot_for_pair_cast_and_sret() {
    use kfrc_body::{CastAbi, CastExtension, CastLane};
    let cast = PassMode::Cast(CastAbi {
        size: 16,
        lanes: [
            Some(CastLane {
                offset: 0,
                bytes: 8,
            }),
            Some(CastLane {
                offset: 8,
                bytes: 8,
            }),
        ],
        consecutive: false,
        pad_i32: false,
        extension: CastExtension::None,
    });
    for mode in [PassMode::Pair, cast, PassMode::Indirect] {
        let mut g = fixture_fields(true, true, 1);
        let ret = g.ids().find(|&n| g.op(n) == Opcode::Return).unwrap();
        let tag = g.input(ret, 1);
        let value = g.input(tag, 0);
        let enumeration = g.rust_type(value).unwrap();
        let word = g.rust_type(tag).unwrap();
        g.set_input(ret, 1, value);
        g.remove_node(tag);
        g.abi.ret = mode;
        g.oracle.record_abi_pair(
            enumeration,
            [
                FieldLayout {
                    ty: word,
                    offset: 0,
                },
                FieldLayout {
                    ty: word,
                    offset: 8,
                },
            ],
        );
        verify_graph(&mut g).unwrap();
        let stats = lower_layout(&mut g).unwrap();
        verify_graph(&mut g).unwrap();
        assert_eq!(stats.owned_transport_phis, 1);
        assert_eq!(stats.abi_slots, 0, "{mode:?}");
        assert_eq!(
            g.ids().filter(|&n| g.op(n) == Opcode::Call).count(),
            0,
            "no return snapshot copy: {mode:?}"
        );
        let ret = g.ids().find(|&n| g.op(n) == Opcode::Return).unwrap();
        assert_eq!(
            g.value_type(g.input(ret, 1)),
            &ValueType::LayoutValue(enumeration)
        );
        for (input, expected) in [Outcome::ReturnPair(7, 0), Outcome::ReturnPair(19, 100)]
            .into_iter()
            .enumerate()
        {
            let actual = Interpreter::new(&g)
                .run(&[input as u64], &mut |name: &str, _: &[u64]| {
                    Err(InterpretError::UnknownCallee(name.into()))
                })
                .unwrap();
            if mode == PassMode::Indirect {
                // The L interpreter validates the typed snapshot and reports its
                // source address. Machine sret copying is covered by real Rust E2E.
                assert!(matches!(actual, Outcome::Return(address) if address != 0));
            } else {
                assert_eq!(actual, expected);
            }
        }
    }
}
