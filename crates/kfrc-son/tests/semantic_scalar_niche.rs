//! A niche and its fieldless payload occupy the same captured scalar bytes.
use kfrc_body::*;
use kfrc_son::{
    CallTarget, ComparePredicate, FunctionGraph, GraphLevel, InterpretError, Interpreter, NodeId,
    Opcode, Outcome, SemanticOutcome, Value, ValueType, lower_layout, verify_graph,
};

fn project(g: &mut FunctionGraph, source: NodeId, slot: u32, ty: ValueType) -> NodeId {
    g.add_node(Opcode::Projection, NodeId::NONE, &[source], slot, ty)
}

fn fixed(
    g: &mut FunctionGraph,
    op: Opcode,
    ctl: &mut NodeId,
    eff: &mut NodeId,
    operands: &[NodeId],
    value: Option<TyId>,
) -> Option<NodeId> {
    let mut inputs = vec![*eff];
    inputs.extend_from_slice(operands);
    let mut outputs = vec![ValueType::Control, ValueType::Effect];
    outputs.extend(value.map(ValueType::Rust));
    let source = g.add_node(op, *ctl, &inputs, 0, ValueType::Tuple(outputs));
    *ctl = project(g, source, 0, ValueType::Control);
    *eff = project(g, source, 1, ValueType::Effect);
    value.map(|ty| project(g, source, 2, ValueType::Rust(ty)))
}

fn fixture(width: IntegerWidth, size: u64, padded: bool, alias: bool) -> FunctionGraph {
    let mut types = RecordedTypes::new();
    let word = types.intern(
        TypeShape::Int {
            width: IntegerWidth::W64,
            signed: false,
        },
        Some(SizeAlign { size: 8, align: 8 }),
    );
    let raw = types.intern(
        TypeShape::Int {
            width,
            signed: false,
        },
        Some(SizeAlign { size, align: size }),
    );
    let signed = types.intern(
        TypeShape::Int {
            width,
            signed: true,
        },
        Some(SizeAlign { size, align: size }),
    );
    let boolean = types.intern(TypeShape::Bool, Some(SizeAlign { size: 1, align: 1 }));
    let unit = types.intern(TypeShape::Unit, Some(SizeAlign { size: 0, align: 1 }));
    let tags = vec![1, (-(1i128 << (size * 8 - 1))) as u128, (-2i128) as u128];
    let payload = types.intern(
        TypeShape::Adt {
            name: "SignedSparsePayload".into(),
        },
        Some(SizeAlign { size, align: size }),
    );
    types.record_aggregate(
        payload,
        AggregateLayout {
            fields: vec![],
            variants: vec![vec![]; tags.len()],
        },
    );
    types.record_semantic_enum(payload, tags.clone());
    types.enums.insert(
        payload,
        EnumLayout {
            tag: Some(FieldLayout {
                ty: signed,
                offset: 0,
            }),
            encoding: TagEncoding::Direct {
                discriminants: tags,
            },
        },
    );
    let outer = types.intern(
        TypeShape::Adt {
            name: format!("NicheSnapshot{size}_{padded}"),
        },
        Some(SizeAlign {
            size: size * if padded { 2 } else { 1 },
            align: size,
        }),
    );
    types.record_aggregate(
        outer,
        AggregateLayout {
            fields: vec![],
            variants: vec![
                vec![FieldLayout {
                    ty: payload,
                    offset: 0,
                }],
                vec![],
                vec![],
            ],
        },
    );
    types.record_semantic_enum(outer, vec![0, 1, 2]);
    types.enums.insert(
        outer,
        EnumLayout {
            tag: Some(FieldLayout { ty: raw, offset: 0 }),
            encoding: TagEncoding::Niche {
                untagged_variant: 0,
                niche_variants: 1..=2,
                niche_start: (1u128 << (size * 8)) - 1,
            },
        },
    );
    let ptr = types.intern(
        TypeShape::RawPointer {
            mutability: Mutability::Mut,
            pointee: outer,
        },
        Some(SizeAlign { size: 8, align: 8 }),
    );
    let abi = FunctionAbi {
        args: vec![PassMode::Direct],
        ret: PassMode::Direct,
        c_variadic: false,
    };
    let mut g = FunctionGraph::new("scalar_niche_snapshot", abi, types);
    g.level = GraphLevel::Specialized;
    g.start = g.add_node(
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
    let start = g.start;
    let mut ctl = project(&mut g, start, 0, ValueType::Control);
    let mut eff = project(&mut g, start, 1, ValueType::Effect);
    let input = project(&mut g, start, 2, ValueType::Rust(word));
    let mut edges = Vec::new();
    let mut values = Vec::new();
    for arm in 0..5 {
        let selected = if arm < 4 {
            let expected = g.add_constant(arm as u128, word);
            let test = g.add_node(
                Opcode::Compare,
                NodeId::NONE,
                &[input, expected],
                ComparePredicate::Equal as u32,
                ValueType::Rust(boolean),
            );
            let branch = g.add_node(
                Opcode::If,
                ctl,
                &[test],
                0,
                ValueType::Tuple(vec![ValueType::Control, ValueType::Control]),
            );
            ctl = project(&mut g, branch, 1, ValueType::Control);
            project(&mut g, branch, 0, ValueType::Control)
        } else {
            ctl
        };
        edges.push(g.add_node(Opcode::Jump, selected, &[], 0, ValueType::Control));
        let fields = if arm < 3 {
            vec![g.add_node(
                Opcode::MakeVariant,
                NodeId::NONE,
                &[],
                arm,
                ValueType::Rust(payload),
            )]
        } else {
            vec![]
        };
        values.push(g.add_node(
            Opcode::MakeVariant,
            NodeId::NONE,
            &fields,
            if arm < 3 { 0 } else { arm - 2 },
            ValueType::Rust(outer),
        ));
    }
    let region = g.add_node(Opcode::Region, NodeId::NONE, &edges, 0, ValueType::Control);
    let mut inputs = vec![region];
    inputs.extend(values);
    let merged = g.add_node(
        Opcode::Phi,
        NodeId::NONE,
        &inputs,
        0,
        ValueType::Rust(outer),
    );
    let source = g.add_node(
        Opcode::LocalSlot,
        NodeId::NONE,
        &[],
        0,
        ValueType::Rust(ptr),
    );
    let destination = if alias {
        source
    } else {
        g.add_node(
            Opcode::LocalSlot,
            NodeId::NONE,
            &[],
            1,
            ValueType::Rust(ptr),
        )
    };
    ctl = region;
    fixed(
        &mut g,
        Opcode::StoreValue,
        &mut ctl,
        &mut eff,
        &[source, merged],
        None,
    );
    let call = g.add_callee(CallTarget {
        symbol: "observe".into(),
        abi: FunctionAbi {
            args: vec![PassMode::Direct],
            ret: PassMode::Ignore,
            c_variadic: false,
        },
    });
    assert_eq!(call, 0);
    fixed(
        &mut g,
        Opcode::Call,
        &mut ctl,
        &mut eff,
        &[source],
        Some(unit),
    );
    let captured = fixed(
        &mut g,
        Opcode::LoadValue,
        &mut ctl,
        &mut eff,
        &[source],
        Some(outer),
    )
    .unwrap();
    fixed(
        &mut g,
        Opcode::Call,
        &mut ctl,
        &mut eff,
        &[source],
        Some(unit),
    );
    let overwrite = g.add_node(
        Opcode::MakeVariant,
        NodeId::NONE,
        &[],
        2,
        ValueType::Rust(outer),
    );
    fixed(
        &mut g,
        Opcode::StoreValue,
        &mut ctl,
        &mut eff,
        &[source, overwrite],
        None,
    );
    fixed(
        &mut g,
        Opcode::StoreValue,
        &mut ctl,
        &mut eff,
        &[destination, captured],
        None,
    );
    let saved = fixed(
        &mut g,
        Opcode::LoadValue,
        &mut ctl,
        &mut eff,
        &[destination],
        Some(outer),
    )
    .unwrap();
    let switch_payload = g.variant_switches.len() as u32;
    g.variant_switches.push(vec![0, 1]);
    let switch = g.add_node(
        Opcode::VariantSwitch,
        ctl,
        &[saved],
        switch_payload,
        ValueType::Tuple(vec![ValueType::Control; 3]),
    );
    for variant in 0..3 {
        let edge = project(&mut g, switch, variant, ValueType::Control);
        let result = if variant == 0 {
            let witness = g.add_node(
                Opcode::VariantWitness,
                edge,
                &[saved, edge],
                0,
                ValueType::Witness,
            );
            let field = g.variant_fields.len() as u32;
            g.variant_fields.push((0, 0));
            let field = g.add_node(
                Opcode::VariantField,
                NodeId::NONE,
                &[saved, witness],
                field,
                ValueType::Rust(payload),
            );
            g.add_node(
                Opcode::Discriminant,
                NodeId::NONE,
                &[field],
                0,
                ValueType::Rust(word),
            )
        } else {
            g.add_constant(400 + u128::from(variant), word)
        };
        g.add_node(Opcode::Return, edge, &[eff, result], 0, ValueType::Control);
    }
    verify_graph(&mut g).unwrap_or_else(|e| panic!("{e}\n{}", g.dump()));
    g
}

#[test]
fn captured_niche_bytes_survive_aliasing_overwrite_and_call_failure() {
    for (width, size) in [
        (IntegerWidth::W8, 1),
        (IntegerWidth::W32, 4),
        (IntegerWidth::W64, 8),
    ] {
        for padded in [false, true] {
            for alias in [false, true] {
                let semantic = fixture(width, size, padded, alias);
                let mut lowered = semantic.clone();
                let stats = lower_layout(&mut lowered).unwrap();
                verify_graph(&mut lowered).unwrap_or_else(|e| panic!("{e}\n{}", lowered.dump()));
                if !padded && std::env::var("KFRC_SCALAR_NICHE_TRANSPORT").as_deref() != Ok("0") {
                    assert!(
                        stats.field_loads <= 2,
                        "no additional per-variant payload reads: size={size}, alias={alias}, stats={stats:?}\n{}",
                        lowered.dump()
                    );
                }
                for selector in 0..8 {
                    let expected = match selector {
                        0 => 1,
                        1 => {
                            (1u64 << (size * 8 - 1))
                                | if size < 8 { u64::MAX << (size * 8) } else { 0 }
                        }
                        2 => u64::MAX - 1,
                        3 => 401,
                        _ => 402,
                    };
                    for fail in [0, 1, 2] {
                        let mut s_calls = 0;
                        let s = Interpreter::new(&semantic).run_values(
                            &[Value::Scalar(selector)],
                            &mut |name: &str, _: &[Value]| {
                                assert_eq!(name, "observe");
                                s_calls += 1;
                                if s_calls == fail {
                                    Err(InterpretError::UnknownCallee("injected".into()))
                                } else {
                                    Ok(Value::Aggregate(vec![]))
                                }
                            },
                        );
                        let mut l_calls = 0;
                        let l = Interpreter::new(&lowered).run(
                            &[selector],
                            &mut |name: &str, _: &[u64]| {
                                assert_eq!(name, "observe");
                                l_calls += 1;
                                if l_calls == fail {
                                    Err(InterpretError::UnknownCallee("injected".into()))
                                } else {
                                    Ok(0)
                                }
                            },
                        );
                        assert_eq!(s_calls, l_calls);
                        if fail == 0 {
                            assert_eq!(
                                s.unwrap(),
                                SemanticOutcome::Return(Value::Scalar(expected))
                            );
                            assert_eq!(l.unwrap(), Outcome::Return(expected), "{}", lowered.dump());
                        } else {
                            assert_eq!(s.unwrap_err(), l.unwrap_err());
                        }
                    }
                }
            }
        }
    }
}
