//! Invalid Rust inputs are exercised only by the semantic interpreter.
use kfrc_body::{
    AggregateLayout, FieldLayout, FunctionAbi, IntegerWidth, PassMode, RecordedTypes,
    ScalarValidRange, ScalarValidity, SizeAlign, TyId, TypeShape,
};
use kfrc_son::{
    FunctionGraph, GraphLevel, InterpretError, Interpreter, NodeId, Opcode, SemanticMemory, Value,
    ValueType,
};

fn word(types: &mut RecordedTypes) -> TyId {
    types.intern(
        TypeShape::Int {
            width: IntegerWidth::W32,
            signed: false,
        },
        Some(SizeAlign { size: 4, align: 4 }),
    )
}
fn nonzero(types: &mut RecordedTypes, field: TyId) -> TyId {
    let ty = types.intern(
        TypeShape::Adt {
            name: "NonZeroWord".into(),
        },
        Some(SizeAlign { size: 4, align: 4 }),
    );
    types.record_aggregate(
        ty,
        AggregateLayout {
            fields: vec![FieldLayout {
                ty: field,
                offset: 0,
            }],
            variants: vec![],
        },
    );
    types.record_scalar_validity(
        ty,
        ScalarValidity::Scalar(ScalarValidRange::new(32, 1, u32::MAX as u128).unwrap()),
    );
    ty
}
fn identity(types: RecordedTypes, input: TyId, output: TyId) -> FunctionGraph {
    let mut g = FunctionGraph::new(
        "validity",
        FunctionAbi {
            args: vec![PassMode::Direct],
            ret: PassMode::Direct,
            c_variadic: false,
        },
        types,
    );
    g.level = GraphLevel::Specialized;
    g.start = g.add_node(
        Opcode::Start,
        NodeId::NONE,
        &[],
        0,
        ValueType::Tuple(vec![
            ValueType::Control,
            ValueType::Effect,
            ValueType::Rust(input),
        ]),
    );
    let mut control = g.add_node(
        Opcode::Projection,
        NodeId::NONE,
        &[g.start],
        0,
        ValueType::Control,
    );
    let mut effect = g.add_node(
        Opcode::Projection,
        NodeId::NONE,
        &[g.start],
        1,
        ValueType::Effect,
    );
    let mut value = g.add_node(
        Opcode::Projection,
        NodeId::NONE,
        &[g.start],
        2,
        ValueType::Rust(input),
    );
    if input != output {
        let transmute = g.add_node(
            Opcode::TransmuteValue,
            control,
            &[effect, value],
            0,
            ValueType::Tuple(vec![
                ValueType::Control,
                ValueType::Effect,
                ValueType::Rust(output),
            ]),
        );
        control = g.add_node(
            Opcode::Projection,
            NodeId::NONE,
            &[transmute],
            0,
            ValueType::Control,
        );
        effect = g.add_node(
            Opcode::Projection,
            NodeId::NONE,
            &[transmute],
            1,
            ValueType::Effect,
        );
        value = g.add_node(
            Opcode::Projection,
            NodeId::NONE,
            &[transmute],
            2,
            ValueType::Rust(output),
        );
    }
    g.add_node(
        Opcode::Return,
        control,
        &[effect, value],
        0,
        ValueType::Control,
    );
    g
}
fn run(g: &FunctionGraph, value: Value) -> Result<kfrc_son::SemanticOutcome, InterpretError> {
    Interpreter::new(g).run_values(&[value], &mut |_: &str, _: &[Value]| unreachable!())
}

#[test]
fn nonzero_entry_and_transmute_reject_zero_without_poisoning_valid_inputs() {
    let mut types = RecordedTypes::new();
    let u32_ = word(&mut types);
    let nz = nonzero(&mut types, u32_);
    let entry = identity(types.clone(), nz, nz);
    assert!(matches!(
        run(&entry, Value::Aggregate(vec![Value::Scalar(0)])),
        Err(InterpretError::UndefinedBehaviour(_))
    ));
    for bits in [1, 7, u32::MAX as u64] {
        assert!(run(&entry, Value::Aggregate(vec![Value::Scalar(bits)])).is_ok());
    }
    let transmute = identity(types, u32_, nz);
    assert!(matches!(
        run(&transmute, Value::Scalar(0)),
        Err(InterpretError::UndefinedBehaviour(_))
    ));
    assert!(run(&transmute, Value::Scalar(1)).is_ok());
}

#[test]
fn typed_load_checks_wrappers_but_raw_and_maybe_uninit_keep_their_own_contract() {
    let mut types = RecordedTypes::new();
    let u32_ = word(&mut types);
    let nz = nonzero(&mut types, u32_);
    let maybe = types.intern(
        TypeShape::Adt {
            name: "MaybeUninitNZ".into(),
        },
        Some(SizeAlign { size: 4, align: 4 }),
    );
    types.record_maybe_uninit(maybe, nz);
    let mut memory = SemanticMemory::new();
    let address = memory.allocate(4, 4, true).unwrap();
    memory.write_bytes(address, &[Some(0); 4]).unwrap();
    assert!(matches!(
        memory.load(&types, address, nz),
        Err(InterpretError::UndefinedBehaviour(_))
    ));
    assert_eq!(
        memory.load(&types, address, u32_).unwrap(),
        Value::Scalar(0)
    );
    assert!(matches!(
        memory.load(&types, address, maybe).unwrap(),
        Value::Uninit { .. }
    ));
    memory.write_bytes(address, &[None; 4]).unwrap();
    assert!(memory.load(&types, address, nz).unwrap().contains_poison());
    assert!(
        !memory
            .load(&types, address, maybe)
            .unwrap()
            .contains_poison()
    );
}

#[test]
fn pair_lanes_and_wrap_ranges_check_the_nominal_type() {
    let mut types = RecordedTypes::new();
    let u32_ = word(&mut types);
    let pair = types.intern(
        TypeShape::Tuple(vec![u32_, u32_]),
        Some(SizeAlign { size: 8, align: 4 }),
    );
    types.record_aggregate(
        pair,
        AggregateLayout {
            fields: vec![
                FieldLayout {
                    ty: u32_,
                    offset: 4,
                },
                FieldLayout {
                    ty: u32_,
                    offset: 0,
                },
            ],
            variants: vec![],
        },
    );
    types.record_scalar_validity(
        pair,
        ScalarValidity::Pair {
            first: Some(ScalarValidRange::new(32, 1, u32::MAX as u128).unwrap()),
            second: Some(ScalarValidRange::new(32, u32::MAX as u128 - 1, 2).unwrap()),
            second_offset: 4,
        },
    );
    let entry = identity(types.clone(), pair, pair);
    assert!(
        run(
            &entry,
            Value::Aggregate(vec![Value::Scalar(2), Value::Scalar(1)])
        )
        .is_ok()
    );
    assert!(matches!(
        run(
            &entry,
            Value::Aggregate(vec![Value::Scalar(3), Value::Scalar(1)])
        ),
        Err(InterpretError::UndefinedBehaviour(_))
    ));
    assert!(matches!(
        run(
            &entry,
            Value::Aggregate(vec![Value::Scalar(2), Value::Scalar(0)])
        ),
        Err(InterpretError::UndefinedBehaviour(_))
    ));
    let mut memory = SemanticMemory::new();
    let address = memory.allocate(8, 4, true).unwrap();
    memory
        .write_bytes(address, &[1, 0, 0, 0, 3, 0, 0, 0].map(Some))
        .unwrap();
    assert!(matches!(
        memory.load(&types, address, pair),
        Err(InterpretError::UndefinedBehaviour(_))
    ));
}

#[test]
fn bool_char_and_nonnull_invalid_decoding_is_ub() {
    let mut types = RecordedTypes::new();
    let bool_ = types.intern(TypeShape::Bool, Some(SizeAlign { size: 1, align: 1 }));
    let char_ = types.intern(TypeShape::Char, Some(SizeAlign { size: 4, align: 4 }));
    let pointer = types.intern(
        TypeShape::RawPointer {
            mutability: kfrc_body::Mutability::Not,
            pointee: bool_,
        },
        Some(SizeAlign { size: 8, align: 8 }),
    );
    let nonnull = types.intern(
        TypeShape::Adt {
            name: "NonNullWord".into(),
        },
        Some(SizeAlign { size: 8, align: 8 }),
    );
    types.record_aggregate(
        nonnull,
        AggregateLayout {
            fields: vec![FieldLayout {
                ty: pointer,
                offset: 0,
            }],
            variants: vec![],
        },
    );
    types.record_scalar_validity(
        nonnull,
        ScalarValidity::Scalar(ScalarValidRange::new(64, 1, u64::MAX as u128).unwrap()),
    );
    for (ty, bytes) in [
        (bool_, 2u64),
        (char_, 0xd800),
        (char_, 0x110000),
        (nonnull, 0),
    ] {
        let mut memory = SemanticMemory::new();
        let address = memory.allocate(8, 8, true).unwrap();
        memory
            .write_bytes(address, &bytes.to_le_bytes().map(Some))
            .unwrap();
        assert!(matches!(
            memory.load(&types, address, ty),
            Err(InterpretError::UndefinedBehaviour(_))
        ));
    }
    assert!(matches!(
        run(&identity(types.clone(), bool_, bool_), Value::Scalar(2)),
        Err(InterpretError::UndefinedBehaviour(_))
    ));
    assert!(matches!(
        run(&identity(types, char_, char_), Value::Scalar(0xd800)),
        Err(InterpretError::UndefinedBehaviour(_))
    ));
}

fn compare_return(
    g: &mut FunctionGraph,
    predicate: kfrc_son::ComparePredicate,
    constant: u128,
) -> NodeId {
    let ret = g.ids().find(|&n| g.op(n) == Opcode::Return).unwrap();
    let source = g.input(ret, 1);
    let ty = g.rust_type(source).unwrap();
    let constant = g.add_constant(constant, ty);
    let bool_ = g
        .oracle
        .intern(TypeShape::Bool, Some(SizeAlign { size: 1, align: 1 }));
    let compare = g.add_node(
        Opcode::Compare,
        NodeId::NONE,
        &[source, constant],
        predicate as u32,
        ValueType::Rust(bool_),
    );
    g.set_input(ret, 1, compare);
    compare
}

#[test]
fn nonzero_transmute_and_typed_field_comparisons_fold_and_preserve_defined_results() {
    use kfrc_son::{ComparePredicate, fold_scalar_ranges};
    for through_field in [false, true] {
        let mut types = RecordedTypes::new();
        let u32_ = word(&mut types);
        let nz = nonzero(&mut types, u32_);
        let mut g = identity(types, nz, if through_field { nz } else { u32_ });
        if through_field {
            let ret = g.ids().find(|&n| g.op(n) == Opcode::Return).unwrap();
            let source = g.input(ret, 1);
            let field = g.add_node(
                Opcode::Field,
                NodeId::NONE,
                &[source],
                0,
                ValueType::Rust(u32_),
            );
            g.set_input(ret, 1, field);
        }
        let compare = compare_return(&mut g, ComparePredicate::NotEqual, 0);
        let before = g.clone();
        assert_eq!(fold_scalar_ranges(&mut g).comparisons_folded, 1);
        assert_eq!(g.op(g.resolve(compare)), Opcode::Const);
        for value in [1, 2, 255, 65536, u32::MAX as u64] {
            let arg = Value::Aggregate(vec![Value::Scalar(value)]);
            assert_eq!(run(&g, arg.clone()), run(&before, arg));
        }
    }
}

#[test]
fn char_cast_upper_bound_folds_but_raw_integer_has_no_char_validity() {
    use kfrc_son::{ComparePredicate, fold_scalar_ranges};
    for typed_char in [false, true] {
        let mut types = RecordedTypes::new();
        let u32_ = word(&mut types);
        let char_ = types.intern(TypeShape::Char, Some(SizeAlign { size: 4, align: 4 }));
        let input = if typed_char { char_ } else { u32_ };
        let mut g = identity(types, input, input);
        if typed_char {
            let ret = g.ids().find(|&n| g.op(n) == Opcode::Return).unwrap();
            let source = g.input(ret, 1);
            let cast = g.add_node(
                Opcode::IntCast,
                NodeId::NONE,
                &[source],
                0,
                ValueType::Rust(u32_),
            );
            g.set_input(ret, 1, cast);
        }
        compare_return(&mut g, ComparePredicate::UnsignedLessEqual, 0x10ffff);
        let before = g.clone();
        assert_eq!(
            fold_scalar_ranges(&mut g).comparisons_folded,
            usize::from(typed_char)
        );
        for value in [0, 65, 0xd7ff, 0xe000, 0x10ffff] {
            assert_eq!(
                run(&g, Value::Scalar(value)),
                run(&before, Value::Scalar(value))
            );
        }
        if !typed_char {
            assert_eq!(
                run(&g, Value::Scalar(u32::MAX as u64)).unwrap(),
                kfrc_son::SemanticOutcome::Return(Value::Scalar(0))
            );
        }
    }
}

#[test]
fn undefined_constrepr_and_typed_load_never_gain_definedness_from_their_type() {
    use kfrc_son::analysis::ranges::RangeFacts;
    let mut types = RecordedTypes::new();
    let u32_ = word(&mut types);
    let nz = nonzero(&mut types, u32_);
    let mut g = identity(types, nz, nz);
    let undefined = g.add_node(Opcode::Undefined, NodeId::NONE, &[], 0, ValueType::Rust(nz));
    let mut range = RangeFacts::new(&g);
    assert!(range.range_of(undefined).is_none());
    drop(range);
    let repr = g.add_constant(1, u32_);
    g.set_opcode(repr, Opcode::ConstRepr);
    let ret = g.ids().find(|&n| g.op(n) == Opcode::Return).unwrap();
    let pointer = g.oracle.intern(
        TypeShape::PlaceAddress { pointee: nz },
        Some(SizeAlign { size: 8, align: 8 }),
    );
    let address = g.add_node(
        Opcode::Undefined,
        NodeId::NONE,
        &[],
        0,
        ValueType::Rust(pointer),
    );
    let load = g.add_node(
        Opcode::LoadValue,
        g.control(ret),
        &[g.input(ret, 0), address],
        0,
        ValueType::Tuple(vec![
            ValueType::Control,
            ValueType::Effect,
            ValueType::Rust(nz),
        ]),
    );
    let loaded = g.add_node(
        Opcode::Projection,
        NodeId::NONE,
        &[load],
        2,
        ValueType::Rust(nz),
    );
    let mut ranges = RangeFacts::new(&g);
    assert!(ranges.range_of(repr).is_none());
    assert!(ranges.range_of(loaded).is_none());
}

#[test]
fn maybe_uninit_raw_decode_does_not_inherit_nonzero() {
    use kfrc_son::{ComparePredicate, fold_scalar_ranges};
    let mut types = RecordedTypes::new();
    let u32_ = word(&mut types);
    let nz = nonzero(&mut types, u32_);
    let maybe = types.intern(
        TypeShape::Adt {
            name: "MaybeUninitNZ".into(),
        },
        Some(SizeAlign { size: 4, align: 4 }),
    );
    types.record_maybe_uninit(maybe, nz);
    let mut g = identity(types, maybe, u32_);
    compare_return(&mut g, ComparePredicate::NotEqual, 0);
    assert_eq!(fold_scalar_ranges(&mut g).comparisons_folded, 0);
    let zero = Value::Uninit {
        bytes: std::sync::Arc::new(vec![Some(0); 4]),
    };
    assert_eq!(
        run(&g, zero).unwrap(),
        kfrc_son::SemanticOutcome::Return(Value::Scalar(0))
    );
}

#[test]
fn casts_masks_and_shifts_have_conservative_bounds_on_all_byte_values() {
    use kfrc_son::analysis::ranges::RangeFacts;
    let mut types = RecordedTypes::new();
    let byte = types.intern(
        TypeShape::Int {
            width: IntegerWidth::W8,
            signed: false,
        },
        Some(SizeAlign { size: 1, align: 1 }),
    );
    let mut g = identity(types, byte, byte);
    let ret = g.ids().find(|&n| g.op(n) == Opcode::Return).unwrap();
    let input = g.input(ret, 1);
    let mask = g.add_constant(31, byte);
    let masked = g.add_node(
        Opcode::BitAnd,
        NodeId::NONE,
        &[input, mask],
        0,
        ValueType::Rust(byte),
    );
    let amount = g.add_constant(3, byte);
    let shifted = g.add_node(
        Opcode::ShiftRightLogical,
        NodeId::NONE,
        &[masked, amount],
        0,
        ValueType::Rust(byte),
    );
    g.set_input(ret, 1, shifted);
    let bound = RangeFacts::new(&g).range_of(shifted).unwrap();
    assert_eq!((bound.min, bound.max), (0, 3));
    for input in 0..=255 {
        let kfrc_son::SemanticOutcome::Return(Value::Scalar(value)) =
            run(&g, Value::Scalar(input)).unwrap()
        else {
            panic!()
        };
        assert_eq!(value, (input & 31) >> 3);
        assert!(bound.min <= value && value <= bound.max);
    }
}

#[test]
fn loop_phi_and_flagged_arithmetic_do_not_self_certify_ranges() {
    use kfrc_son::analysis::ranges::RangeFacts;
    let mut types = RecordedTypes::new();
    let u32_ = word(&mut types);
    let mut g = identity(types, u32_, u32_);
    let ret = g.ids().find(|&n| g.op(n) == Opcode::Return).unwrap();
    let input = g.input(ret, 1);
    let region = g.add_node(
        Opcode::Region,
        NodeId::NONE,
        &[g.control(ret), g.control(ret)],
        0,
        ValueType::Control,
    );
    let phi = g.add_node(
        Opcode::Phi,
        NodeId::NONE,
        &[region, input, input],
        0,
        ValueType::Rust(u32_),
    );
    g.set_input(phi, 2, phi);
    let add = g.add_node(
        Opcode::Add,
        NodeId::NONE,
        &[input, input],
        1,
        ValueType::Rust(u32_),
    );
    let mut facts = RangeFacts::new(&g);
    assert!(facts.range_of(phi).is_none());
    assert!(facts.range_of(add).is_none());
}

#[test]
fn union_storage_does_not_retain_an_inner_or_stale_nominal_constraint() {
    let mut types = RecordedTypes::new();
    let u32_ = word(&mut types);
    let restricted = nonzero(&mut types, u32_);
    // Registering the actual union identity invalidates any earlier bootstrap
    // scalar fact. The raw u32 field never gains the restricted field's range.
    types.record_union(
        restricted,
        vec![FieldLayout {
            ty: u32_,
            offset: 0,
        }],
    );
    assert!(types.scalar_validity(restricted).is_none());
    assert!(types.scalar_validity(u32_).is_none());
}

#[test]
fn folding_a_transmuted_nonzero_check_preserves_the_fixed_ub_observation() {
    use kfrc_son::ComparePredicate;
    for (poison_source, seams) in [(false, false), (false, true), (true, false), (true, true)] {
        let mut types = RecordedTypes::new();
        let u32_ = word(&mut types);
        let nz = nonzero(&mut types, u32_);
        let mut g = identity(types, u32_, nz);
        let transmute = g
            .ids()
            .find(|&n| g.op(n) == Opcode::TransmuteValue)
            .unwrap();
        if poison_source {
            let poison = g.add_node(
                Opcode::Undefined,
                NodeId::NONE,
                &[],
                0,
                ValueType::Rust(u32_),
            );
            g.set_input(transmute, 1, poison);
        }
        let ret = g.ids().find(|&n| g.op(n) == Opcode::Return).unwrap();
        let value = g.input(ret, 1);
        let field = g.add_node(
            Opcode::Field,
            NodeId::NONE,
            &[value],
            0,
            ValueType::Rust(u32_),
        );
        g.set_input(ret, 1, field);
        compare_return(&mut g, ComparePredicate::NotEqual, 0);
        let before = g.clone();
        let stats = if seams {
            let seeds: Vec<_> = g.ids().collect();
            kfrc_son::optimize_specialized_seams(&mut g, &seeds, &[]).unwrap()
        } else {
            kfrc_son::optimize_specialized_graph(&mut g).unwrap()
        };
        assert!(stats.range_comparisons_folded > 0);
        g.compact();
        assert_eq!(
            g.ids()
                .filter(|&n| g.op(n) == Opcode::TransmuteValue)
                .count(),
            1
        );
        for value in [0, 1, u32::MAX as u64] {
            let old = run(&before, Value::Scalar(value));
            let new = run(&g, Value::Scalar(value));
            if poison_source {
                assert!(matches!(old, Err(InterpretError::PoisonObserved(_))));
                assert!(matches!(new, Err(InterpretError::PoisonObserved(_))));
            } else if value == 0 {
                assert!(matches!(old, Err(InterpretError::UndefinedBehaviour(_))));
                assert!(matches!(new, Err(InterpretError::UndefinedBehaviour(_))));
            } else {
                assert_eq!(new, old);
            }
        }
    }
}

#[test]
fn range_folding_keeps_if_and_assume_predicates_used_as_variant_evidence() {
    use kfrc_son::{ComparePredicate, fold_scalar_ranges, verify_graph};
    for assumption in [false, true] {
        let mut types = RecordedTypes::new();
        let word = word(&mut types);
        let enumeration = types.intern(
            TypeShape::Adt {
                name: "OneVariantWithPayload".into(),
            },
            Some(SizeAlign { size: 4, align: 4 }),
        );
        types.record_aggregate(
            enumeration,
            AggregateLayout {
                fields: vec![],
                variants: vec![vec![FieldLayout {
                    ty: word,
                    offset: 0,
                }]],
            },
        );
        types.record_semantic_enum(enumeration, vec![0]);
        types.enums.insert(
            enumeration,
            kfrc_body::EnumLayout {
                tag: None,
                encoding: kfrc_body::TagEncoding::Single {
                    variant: 0,
                    discriminant: 0,
                },
            },
        );
        let mut g = identity(types, enumeration, enumeration);
        let ret = g.ids().find(|&n| g.op(n) == Opcode::Return).unwrap();
        let control = g.control(ret);
        let mut effect = g.input(ret, 0);
        let value = g.input(ret, 1);
        g.remove_node(ret);
        let tag = g.add_node(
            Opcode::VariantOf,
            NodeId::NONE,
            &[value],
            0,
            ValueType::Rust(word),
        );
        let zero = g.add_constant(0, word);
        let bool_ = g
            .oracle
            .intern(TypeShape::Bool, Some(SizeAlign { size: 1, align: 1 }));
        let compare = g.add_node(
            Opcode::Compare,
            NodeId::NONE,
            &[tag, zero],
            ComparePredicate::Equal as u32,
            ValueType::Rust(bool_),
        );
        let operation = if assumption {
            g.add_node(
                Opcode::Assume,
                control,
                &[effect, compare],
                0,
                ValueType::Tuple(vec![ValueType::Control, ValueType::Effect]),
            )
        } else {
            g.add_node(
                Opcode::If,
                control,
                &[compare],
                0,
                ValueType::Tuple(vec![ValueType::Control, ValueType::Control]),
            )
        };
        let proof = g.add_node(
            Opcode::Projection,
            NodeId::NONE,
            &[operation],
            0,
            ValueType::Control,
        );
        if assumption {
            effect = g.add_node(
                Opcode::Projection,
                NodeId::NONE,
                &[operation],
                1,
                ValueType::Effect,
            );
        } else {
            let otherwise = g.add_node(
                Opcode::Projection,
                NodeId::NONE,
                &[operation],
                1,
                ValueType::Control,
            );
            g.add_node(
                Opcode::Return,
                otherwise,
                &[effect, zero],
                0,
                ValueType::Control,
            );
        }
        let witness = g.add_node(
            Opcode::VariantWitness,
            proof,
            &[value, proof],
            0,
            ValueType::Witness,
        );
        g.variant_fields.push((0, 0));
        let field = g.add_node(
            Opcode::VariantField,
            NodeId::NONE,
            &[value, witness],
            0,
            ValueType::Rust(word),
        );
        g.add_node(
            Opcode::Return,
            proof,
            &[effect, field],
            0,
            ValueType::Control,
        );
        verify_graph(&mut g).unwrap();
        assert_eq!(
            kfrc_son::analysis::ranges::RangeFacts::new(&g).compare(compare),
            Some(true)
        );
        assert_eq!(fold_scalar_ranges(&mut g).comparisons_folded, 0);
        verify_graph(&mut g).unwrap();
        let arg = Value::Variant {
            variant: 0,
            fields: vec![Value::Scalar(19)],
        };
        assert_eq!(
            run(&g, arg).unwrap(),
            kfrc_son::SemanticOutcome::Return(Value::Scalar(19))
        );
    }
}

#[test]
fn nullable_raw_pointer_and_tuple_do_not_inherit_a_nominal_callback_range() {
    use kfrc_son::{ComparePredicate, fold_scalar_ranges};
    for tuple in [false, true] {
        let mut types = RecordedTypes::new();
        let unit = types.intern(TypeShape::Unit, Some(SizeAlign { size: 0, align: 1 }));
        let pointer = types.intern(
            TypeShape::RawPointer {
                mutability: kfrc_body::Mutability::Not,
                pointee: unit,
            },
            Some(SizeAlign { size: 8, align: 8 }),
        );
        // The converter's erased FnPtr has exactly this pointer shape. An
        // enclosing nominal callback holder still has its independent range.
        let callback = types.intern(
            TypeShape::Adt {
                name: "CallbackHolder#nominal".into(),
            },
            Some(SizeAlign { size: 8, align: 8 }),
        );
        types.record_aggregate(
            callback,
            AggregateLayout {
                fields: vec![FieldLayout {
                    ty: pointer,
                    offset: 0,
                }],
                variants: vec![],
            },
        );
        types.record_scalar_validity(
            callback,
            ScalarValidity::Scalar(ScalarValidRange::new(64, 1, u64::MAX as u128).unwrap()),
        );
        assert!(types.scalar_validity(pointer).is_none());
        let word = types.intern(
            TypeShape::Int {
                width: IntegerWidth::Pointer,
                signed: false,
            },
            Some(SizeAlign { size: 8, align: 8 }),
        );
        let pair = types.intern(
            TypeShape::Tuple(vec![pointer, word]),
            Some(SizeAlign { size: 16, align: 8 }),
        );
        types.record_aggregate(
            pair,
            AggregateLayout {
                fields: vec![
                    FieldLayout {
                        ty: pointer,
                        offset: 0,
                    },
                    FieldLayout {
                        ty: word,
                        offset: 8,
                    },
                ],
                variants: vec![],
            },
        );
        assert!(types.scalar_validity(pair).is_none());
        let input = if tuple { pair } else { pointer };
        let mut g = identity(types, input, input);
        if tuple {
            g.abi.args[0] = PassMode::Pair;
            let ret = g.ids().find(|&n| g.op(n) == Opcode::Return).unwrap();
            let value = g.input(ret, 1);
            let field = g.add_node(
                Opcode::Field,
                NodeId::NONE,
                &[value],
                0,
                ValueType::Rust(pointer),
            );
            g.set_input(ret, 1, field);
        }
        compare_return(&mut g, ComparePredicate::Equal, 0);
        assert_eq!(fold_scalar_ranges(&mut g).comparisons_folded, 0);
        for bits in [0, 1, 0x10000] {
            let value = if tuple {
                Value::Aggregate(vec![Value::Scalar(bits), Value::Scalar(7)])
            } else {
                Value::Scalar(bits)
            };
            assert_eq!(
                run(&g, value).unwrap(),
                kfrc_son::SemanticOutcome::Return(Value::Scalar(u64::from(bits == 0)))
            );
        }
    }
}
