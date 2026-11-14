use super::*;

fn enum_type(f: &mut Fixture, uninhabited_variant: bool) -> TyId {
    let never = uninhabited_variant.then(|| {
        f.graph
            .oracle
            .intern(TypeShape::Never, Some(SizeAlign { size: 0, align: 1 }))
    });
    let enumeration = f.graph.oracle.intern(
        TypeShape::Adt {
            name: "DseChoice".into(),
        },
        Some(SizeAlign {
            size: if uninhabited_variant { 8 } else { 16 },
            align: 8,
        }),
    );
    f.graph.oracle.record_aggregate(
        enumeration,
        AggregateLayout {
            fields: vec![],
            variants: if let Some(never) = never {
                // A genuine Good(u64)/Bad(!) shape: only Good is inhabited,
                // and the representation has the size of its u64 payload.
                vec![
                    vec![FieldLayout {
                        ty: f.word,
                        offset: 0,
                    }],
                    vec![FieldLayout {
                        ty: never,
                        offset: 0,
                    }],
                ]
            } else {
                vec![
                    vec![],
                    vec![FieldLayout {
                        ty: f.word,
                        offset: 8,
                    }],
                ]
            },
        },
    );
    f.graph.oracle.record_semantic_enum(enumeration, vec![0, 1]);
    if uninhabited_variant {
        f.graph
            .oracle
            .record_semantic_inhabited_variants(enumeration, vec![0]);
    }
    f.graph.oracle.enums.insert(
        enumeration,
        EnumLayout {
            tag: (!uninhabited_variant).then_some(FieldLayout {
                ty: f.word,
                offset: 0,
            }),
            encoding: if uninhabited_variant {
                TagEncoding::Single {
                    variant: 0,
                    discriminant: 0,
                }
            } else {
                TagEncoding::Direct {
                    discriminants: vec![0, 1],
                }
            },
        },
    );
    enumeration
}

#[test]
fn enum_stores_do_not_count_as_complete_overwrites_or_nontrapping_intermediate_writes() {
    for branched in [false, true] {
        for bad_first in [false, true] {
            let mut f = Fixture::new(usize::from(branched), true);
            let enumeration = enum_type(&mut f, true);
            let address = f.cast(f.address, enumeration);
            let one = f.graph.add_constant(1, f.word);
            let never = f
                .graph
                .oracle
                .semantic()
                .field_type(enumeration, Some(1), 0)
                .unwrap();
            let impossible = f.poison(never);
            let invalid = f.graph.add_node(
                Opcode::MakeVariant,
                NodeId::NONE,
                &[impossible],
                1,
                ValueType::Rust(enumeration),
            );
            let good = f.graph.add_node(
                Opcode::MakeVariant,
                NodeId::NONE,
                &[one],
                0,
                ValueType::Rust(enumeration),
            );
            let (first, last) = if bad_first {
                (invalid, good)
            } else {
                (good, invalid)
            };
            let old = paired_writes(&mut f, address, first, last, branched);
            let original = f.graph.clone();
            assert_eq!(f.check(old, false).stores_removed, 0);
            let input = vec![Value::Scalar(0); usize::from(branched)];
            let before = execute(&original, &input, false);
            assert_undefined(&before);
            assert_eq!(execute(&f.graph, &input, false), before);
        }
    }
}

#[test]
fn a_disjoint_uninhabited_enum_write_preserves_the_prior_scalar_memory_on_error() {
    for branched in [false, true] {
        let mut f = Fixture::new(usize::from(branched), true);
        let enumeration = enum_type(&mut f, true);
        let left = f.field(f.address, 0);
        let right = f.field(f.address, 1);
        let right = f.cast(right, enumeration);
        let never = f
            .graph
            .oracle
            .semantic()
            .field_type(enumeration, Some(1), 0)
            .unwrap();
        let impossible = f.poison(never);
        let invalid = f.graph.add_node(
            Opcode::MakeVariant,
            NodeId::NONE,
            &[impossible],
            1,
            ValueType::Rust(enumeration),
        );
        let one = f.graph.add_constant(1, f.word);
        let two = f.graph.add_constant(2, f.word);
        let old = f.store(left, one);
        if branched {
            let states = f.branch(f.conditions[0]);
            f.enter(states[0]);
            f.store(left, two);
            f.load_return(left);
            f.enter(states[1]);
        }
        f.store(right, invalid);
        f.store(left, two);
        f.load_return(left);
        let original = f.graph.clone();
        assert_eq!(f.check(old, false).stores_removed, 0);
        let input = vec![Value::Scalar(0); usize::from(branched)];
        let before = execute(&original, &input, false);
        assert_undefined(&before);
        assert_eq!(before.bytes[..8], 1u64.to_le_bytes().map(Some));
        assert_eq!(execute(&f.graph, &input, false), before);
    }
}

#[test]
fn changing_enum_variants_preserves_bytes_outside_the_final_active_payload() {
    for branched in [false, true] {
        let mut f = Fixture::new(usize::from(branched), true);
        let enumeration = enum_type(&mut f, false);
        let address = f.cast(f.address, enumeration);
        let payload = f.graph.add_constant(91, f.word);
        let first = f.graph.add_node(
            Opcode::MakeVariant,
            NodeId::NONE,
            &[payload],
            1,
            ValueType::Rust(enumeration),
        );
        let last = f.graph.add_node(
            Opcode::MakeVariant,
            NodeId::NONE,
            &[],
            0,
            ValueType::Rust(enumeration),
        );
        let old = paired_writes(&mut f, address, first, last, branched);
        assert_eq!(f.check(old, false).stores_removed, 0);
        let input = vec![Value::Scalar(0); usize::from(branched)];
        let execution = execute(&f.graph, &input, false);
        assert_eq!(
            execution.result,
            Ok(SemanticOutcome::Return(Value::Scalar(1)))
        );
        assert_eq!(execution.bytes[8..], 91u64.to_le_bytes().map(Some));
    }
}

#[test]
fn invalid_enum_tag_constrepr_is_not_hidden_by_a_scalar_variant_query() {
    for branched in [false, true] {
        let mut f = Fixture::new(usize::from(branched), true);
        let enumeration = enum_type(&mut f, false);
        let bad = representation(&mut f, 2, enumeration);
        let query = f.graph.add_node(
            Opcode::VariantOf,
            NodeId::NONE,
            &[bad],
            0,
            ValueType::Rust(f.word),
        );
        let left = f.field(f.address, 0);
        let one = f.graph.add_constant(1, f.word);
        let old = paired_writes(&mut f, left, query, one, branched);
        assert_eq!(f.check(old, false).stores_removed, 0);
        let input = vec![Value::Scalar(0); usize::from(branched)];
        let execution = execute(&f.graph, &input, false);
        assert_undefined(&execution);
        assert_eq!(execution.bytes, vec![Some(0x5a); 16]);
    }
}
