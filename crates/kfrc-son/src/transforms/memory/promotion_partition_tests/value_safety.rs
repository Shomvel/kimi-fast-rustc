use super::*;

#[test]
fn deleting_a_write_keeps_representation_operand_evaluation() {
    for overwritten in [false, true] {
        let mut f = Fixture::new();
        let slot = f.slot(0, f.pair);
        initialize_pair(&mut f, slot);
        let first = f.field(slot, 0, f.word);
        let second = f.field(slot, 1, f.word);
        let bad = f.graph.add_repr_constant(2, f.boolean);
        let value = f.graph.add_node(
            Opcode::IntCast,
            NodeId::NONE,
            &[bad],
            0,
            ValueType::Rust(f.word),
        );
        f.store(first, value);
        if overwritten {
            f.store(first, f.limit);
        }
        let retained = narrow(&mut f, second, IntegerWidth::W32);
        let result = if overwritten {
            let current = f.load(first, f.word);
            add(&mut f, current, retained)
        } else {
            retained
        };
        f.ret(result);
        verify_graph(&mut f.graph).unwrap();
        assert_eq!(
            execute(&f.graph, false, 17),
            Err(InterpretError::UndefinedBehaviour(
                "invalid bool in memory".into()
            ))
        );
        let stats = assert_equivalent(&mut f, &[(false, 17)]);
        assert_eq!(stats.stores_removed, 0);
    }
}

#[test]
fn unused_typed_load_keeps_char_and_nominal_validity_checks() {
    use kfrc_body::{ScalarValidRange, ScalarValidity};

    for nominal in [false, true] {
        let mut f = Fixture::new();
        let ty = f.graph.oracle.intern(
            if nominal {
                TypeShape::Int {
                    width: IntegerWidth::W32,
                    signed: false,
                }
            } else {
                TypeShape::Char
            },
            Some(SizeAlign { size: 4, align: 4 }),
        );
        if nominal {
            f.graph.oracle.record_scalar_validity(
                ty,
                ScalarValidity::Scalar(ScalarValidRange::new(32, 1, u32::MAX as u128).unwrap()),
            );
        }
        let word = f.word;
        let pair = f.graph.oracle.intern(
            TypeShape::Tuple(vec![ty, word]),
            Some(SizeAlign { size: 16, align: 8 }),
        );
        f.graph.oracle.record_aggregate(
            pair,
            AggregateLayout {
                fields: vec![
                    FieldLayout { ty, offset: 0 },
                    FieldLayout {
                        ty: word,
                        offset: 8,
                    },
                ],
                variants: vec![],
            },
        );
        let slot = f.slot(0, pair);
        let first = f.field(slot, 0, ty);
        let second = f.field(slot, 1, word);
        let bad = f.graph.add_constant(if nominal { 0 } else { 0xd800 }, ty);
        f.store(first, bad);
        f.store(second, f.limit);
        f.load(first, ty);
        let retained = narrow(&mut f, second, IntegerWidth::W32);
        f.ret(retained);
        verify_graph(&mut f.graph).unwrap();
        assert!(matches!(
            execute(&f.graph, false, 17),
            Err(InterpretError::UndefinedBehaviour(_))
        ));
        let stats = assert_equivalent(&mut f, &[(false, 17)]);
        assert_eq!((stats.loads_removed, stats.stores_removed), (0, 0));
    }
}

#[test]
fn a_whole_promoted_load_is_not_an_already_executed_operand() {
    for (whole_local, partial_local) in [(0, 1), (1, 0)] {
        let mut f = Fixture::new();
        let whole = f.slot(whole_local, f.word);
        let partial = f.slot(partial_local, f.pair);
        let bad = f.graph.add_repr_constant(2, f.boolean);
        let value = f.graph.add_node(
            Opcode::IntCast,
            NodeId::NONE,
            &[bad],
            0,
            ValueType::Rust(f.word),
        );
        f.store(whole, value);
        let loaded = f.load(whole, f.word);
        initialize_pair(&mut f, partial);
        let first = f.field(partial, 0, f.word);
        let second = f.field(partial, 1, f.word);
        f.store(first, loaded);
        let retained = narrow(&mut f, second, IntegerWidth::W32);
        f.ret(retained);
        verify_graph(&mut f.graph).unwrap();
        assert!(matches!(
            execute(&f.graph, false, 17),
            Err(InterpretError::UndefinedBehaviour(_))
        ));
        let stats = assert_equivalent(&mut f, &[(false, 17)]);
        assert_eq!(stats.slots_promoted, 1);
        assert_eq!(stats.partitions_promoted, 0);
    }
}

#[test]
fn logical_fields_still_require_in_bounds_aligned_accesses() {
    for out_of_bounds in [false, true] {
        let mut f = Fixture::new();
        let word = f.word;
        let packed = f.graph.oracle.intern(
            TypeShape::Adt {
                name: "PackedForPartialPromotion".into(),
            },
            Some(SizeAlign { size: 17, align: 1 }),
        );
        f.graph.oracle.record_aggregate(
            packed,
            AggregateLayout {
                fields: vec![
                    FieldLayout {
                        ty: word,
                        offset: if out_of_bounds { 24 } else { 1 },
                    },
                    FieldLayout {
                        ty: word,
                        offset: 9,
                    },
                ],
                variants: vec![],
            },
        );
        let slot = f.slot(0, packed);
        let first = f.field(slot, 0, word);
        let second = f.field(slot, 1, word);
        f.store(first, f.limit);
        f.load(first, word);
        let retained = narrow(&mut f, second, IntegerWidth::W8);
        f.ret(retained);
        verify_graph(&mut f.graph).unwrap();
        assert!(matches!(
            execute(&f.graph, false, 17),
            Err(InterpretError::UndefinedBehaviour(_))
        ));
        let stats = assert_equivalent(&mut f, &[(false, 17)]);
        assert_eq!((stats.loads_removed, stats.stores_removed), (0, 0));
    }
}

#[test]
fn retained_aggregate_writes_allow_a_smaller_scalar_partition() {
    let mut f = Fixture::new();
    let word = f.word;
    let inner = f.pair;
    let outer = f.graph.oracle.intern(
        TypeShape::Tuple(vec![inner, word]),
        Some(SizeAlign { size: 24, align: 8 }),
    );
    f.graph.oracle.record_aggregate(
        outer,
        AggregateLayout {
            fields: vec![
                FieldLayout {
                    ty: inner,
                    offset: 0,
                },
                FieldLayout {
                    ty: word,
                    offset: 16,
                },
            ],
            variants: vec![],
        },
    );
    let slot = f.slot(0, outer);
    let aggregate = f.field(slot, 0, inner);
    initialize_pair(&mut f, aggregate);
    let selected = f.field(aggregate, 0, word);
    let other = f.field(slot, 1, word);
    f.store(other, f.limit);
    let old = f.load(selected, word);
    f.store(selected, f.limit);
    let retained = narrow(&mut f, other, IntegerWidth::W32);
    let result = add(&mut f, old, retained);
    f.ret(result);
    let stats = assert_equivalent(&mut f, &samples());
    assert_eq!(stats.partitions_promoted, 1);
    assert_eq!((stats.loads_removed, stats.stores_removed), (1, 1));
}

#[test]
fn dead_poison_writes_do_not_introduce_an_observation() {
    let mut f = Fixture::new();
    let slot = f.slot(0, f.pair);
    initialize_pair(&mut f, slot);
    let first = f.field(slot, 0, f.word);
    let second = f.field(slot, 1, f.word);
    let poison = f.graph.add_node(
        Opcode::Undefined,
        NodeId::NONE,
        &[],
        0,
        ValueType::Rust(f.word),
    );
    f.store(first, poison);
    f.load(first, f.word);
    let retained = narrow(&mut f, second, IntegerWidth::W32);
    f.ret(retained);
    let stats = assert_equivalent(&mut f, &samples());
    assert_eq!(stats.partitions_promoted, 1);
    assert_eq!((stats.loads_removed, stats.stores_removed), (1, 1));
    assert!(execute(&f.graph, false, 17).is_ok());
}

#[test]
fn logical_fields_do_not_hide_overlapping_recorded_layouts() {
    for enclosing_store in [false, true] {
        let mut f = Fixture::new();
        let word = f.word;
        let triple = f.graph.oracle.intern(
            TypeShape::Tuple(vec![word; 3]),
            Some(SizeAlign { size: 24, align: 8 }),
        );
        f.graph.oracle.record_aggregate(
            triple,
            AggregateLayout {
                fields: vec![
                    FieldLayout {
                        ty: word,
                        offset: 0,
                    },
                    FieldLayout {
                        ty: word,
                        offset: 0,
                    },
                    FieldLayout {
                        ty: word,
                        offset: 16,
                    },
                ],
                variants: vec![],
            },
        );
        let slot = f.slot(0, triple);
        let first = f.field(slot, 0, word);
        let second = f.field(slot, 1, word);
        let last = f.field(slot, 2, word);
        let one = f.graph.add_constant(1, word);
        let two = f.graph.add_constant(2, word);
        let three = f.graph.add_constant(3, word);
        if enclosing_store {
            let initial = f.graph.add_node(
                Opcode::Aggregate,
                NodeId::NONE,
                &[one, two, three],
                0,
                ValueType::Rust(triple),
            );
            f.store(slot, initial);
        } else {
            f.store(first, one);
            f.store(second, two);
            f.store(last, three);
        }
        let current = f.load(first, word);
        let retained = narrow(&mut f, last, IntegerWidth::W32);
        let result = add(&mut f, current, retained);
        f.ret(result);
        assert_eq!(
            execute(&f.graph, false, 17),
            Ok(SemanticOutcome::Return(Value::Scalar(5)))
        );
        let stats = assert_equivalent(&mut f, &[(false, 17)]);
        assert_eq!(stats.partitions_promoted, 0);
    }
}

#[test]
fn a_phi_operand_remains_observable_after_control_cleanup() {
    let mut f = Fixture::new();
    let slot = f.slot(0, f.pair);
    initialize_pair(&mut f, slot);
    let first = f.field(slot, 0, f.word);
    let second = f.field(slot, 1, f.word);
    let retained = narrow(&mut f, second, IntegerWidth::W32);
    let incoming_effect = f.effect;
    let (yes, no) = f.branch(f.condition);
    f.control = yes;
    let yes_jump = f.jump();
    f.control = no;
    let no_jump = f.jump();
    let region = f.graph.add_node(
        Opcode::Region,
        NodeId::NONE,
        &[yes_jump, no_jump],
        0,
        ValueType::Control,
    );
    f.control = region;
    f.effect = f.graph.add_node(
        Opcode::Phi,
        NodeId::NONE,
        &[region, incoming_effect, incoming_effect],
        0,
        ValueType::Effect,
    );
    let bad = f.graph.add_repr_constant(2, f.boolean);
    let value = f.graph.add_node(
        Opcode::IntCast,
        NodeId::NONE,
        &[bad],
        0,
        ValueType::Rust(f.word),
    );
    let phi = f.graph.add_node(
        Opcode::Phi,
        NodeId::NONE,
        &[region, value, f.limit],
        0,
        ValueType::Rust(f.word),
    );
    f.store(first, phi);
    f.ret(retained);
    verify_graph(&mut f.graph).unwrap();
    let before = execute(&f.graph, true, 17);
    assert!(matches!(before, Err(InterpretError::UndefinedBehaviour(_))));
    run(&mut f.graph, None);
    crate::transforms::control::simplify::run(&mut f.graph).unwrap();
    verify_graph(&mut f.graph).unwrap();
    assert_eq!(execute(&f.graph, true, 17), before);
}
