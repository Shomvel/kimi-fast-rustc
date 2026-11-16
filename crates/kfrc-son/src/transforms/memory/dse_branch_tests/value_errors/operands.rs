use super::*;

#[test]
fn ordinary_scalar_expressions_and_poison_creation_still_allow_complete_overwrites() {
    for branched in [false, true] {
        for expression in 0..6 {
            let mut f = Fixture::new(usize::from(branched), true);
            let left = f.field(f.address, 0);
            let one = f.graph.add_constant(1, f.word);
            let value = match expression {
                0 => f.graph.add_node(
                    Opcode::Add,
                    NodeId::NONE,
                    &[one, one],
                    0,
                    ValueType::Rust(f.word),
                ),
                1 => {
                    let word = f.word;
                    let poison = f.poison(word);
                    f.graph.add_node(
                        Opcode::Add,
                        NodeId::NONE,
                        &[poison, one],
                        0,
                        ValueType::Rust(word),
                    )
                }
                2 => {
                    let max = f.graph.add_constant(u64::MAX as u128, f.word);
                    f.graph.add_node(
                        Opcode::Add,
                        NodeId::NONE,
                        &[max, one],
                        crate::ArithmeticFlags::NUW,
                        ValueType::Rust(f.word),
                    )
                }
                3 => {
                    let pair = f.graph.add_node(
                        Opcode::Aggregate,
                        NodeId::NONE,
                        &[one, one],
                        0,
                        ValueType::Rust(f.pair),
                    );
                    f.graph.add_node(
                        Opcode::Field,
                        NodeId::NONE,
                        &[pair],
                        1,
                        ValueType::Rust(f.word),
                    )
                }
                4 => {
                    let word = f.graph.oracle.intern(
                        TypeShape::Int {
                            width: IntegerWidth::W32,
                            signed: false,
                        },
                        Some(SizeAlign { size: 4, align: 4 }),
                    );
                    let zero = f.graph.add_constant(0, f.word);
                    let bits = f.graph.add_node(
                        Opcode::BitCount,
                        NodeId::NONE,
                        &[zero],
                        crate::IntegerIntrinsicPayload::LEADING_ZEROS
                            | crate::IntegerIntrinsicPayload::ZERO_POISON,
                        ValueType::Rust(word),
                    );
                    f.graph.add_node(
                        Opcode::IntCast,
                        NodeId::NONE,
                        &[bits],
                        0,
                        ValueType::Rust(f.word),
                    )
                }
                _ => {
                    let array = f.graph.oracle.intern(
                        TypeShape::Array {
                            element: f.word,
                            length: 0,
                        },
                        Some(SizeAlign { size: 0, align: 8 }),
                    );
                    let word = f.word;
                    let boolean = f.boolean;
                    let invalid = representation(&mut f, 2, boolean);
                    let bad = f.graph.add_node(
                        Opcode::IntCast,
                        NodeId::NONE,
                        &[invalid],
                        0,
                        ValueType::Rust(word),
                    );
                    let repeat = f.graph.add_node(
                        Opcode::Repeat,
                        NodeId::NONE,
                        &[bad],
                        0,
                        ValueType::Rust(array),
                    );
                    let pair = f.graph.oracle.intern(
                        TypeShape::Tuple(vec![array, word]),
                        Some(SizeAlign { size: 8, align: 8 }),
                    );
                    f.graph.oracle.record_aggregate(
                        pair,
                        AggregateLayout {
                            fields: vec![
                                FieldLayout {
                                    ty: array,
                                    offset: 0,
                                },
                                FieldLayout {
                                    ty: word,
                                    offset: 0,
                                },
                            ],
                            variants: vec![],
                        },
                    );
                    let pair = f.graph.add_node(
                        Opcode::Aggregate,
                        NodeId::NONE,
                        &[repeat, one],
                        0,
                        ValueType::Rust(pair),
                    );
                    f.graph.add_node(
                        Opcode::Field,
                        NodeId::NONE,
                        &[pair],
                        1,
                        ValueType::Rust(word),
                    )
                }
            };
            let old = paired_writes(&mut f, left, value, one, branched);
            assert_eq!(f.check(old, true).stores_removed, 1);
        }
    }
}

#[test]
fn poison_scalar_stores_remain_deletable_and_final_poison_is_still_observed() {
    for branched in [false, true] {
        for final_poison in [false, true] {
            let mut f = Fixture::new(usize::from(branched), true);
            let left = f.field(f.address, 0);
            let word = f.word;
            let poison = f.poison(word);
            let one = f.graph.add_constant(1, word);
            let old = f.store(left, if final_poison { one } else { poison });
            let value = if final_poison { poison } else { one };
            if branched {
                let states = f.branch(f.conditions[0]);
                f.overwrite_arms(states, left, value);
            } else {
                f.store(left, value);
                f.load_return(left);
            }
            assert_eq!(f.check(old, true).stores_removed, 1);
            let input = vec![Value::Scalar(0); usize::from(branched)];
            let execution = execute(&f.graph, &input, false);
            if final_poison {
                assert!(matches!(
                    execution.result,
                    Err(InterpretError::PoisonObserved(_))
                ));
                assert_eq!(execution.bytes[..8], [None; 8]);
            } else {
                assert_eq!(
                    execution.result,
                    Ok(SemanticOutcome::Return(Value::Scalar(1)))
                );
            }
        }
    }
}

#[test]
fn start_phi_fixed_result_and_anchored_values_keep_their_existing_execution_sites() {
    for source in 0..5 {
        let mut f = Fixture::new(1, true);
        let left = f.field(f.address, 0);
        let right = f.field(f.address, 1);
        let one = f.graph.add_constant(1, f.word);
        let value = match source {
            0 => f.graph.add_node(
                Opcode::IntCast,
                NodeId::NONE,
                &[f.conditions[0]],
                0,
                ValueType::Rust(f.word),
            ),
            1 => f
                .fixed(Opcode::LoadValue, &[right], Some(f.word))
                .1
                .unwrap(),
            2 | 4 => {
                let incoming = if source == 4 {
                    let boolean = f.boolean;
                    let bad = representation(&mut f, 2, boolean);
                    f.graph.add_node(
                        Opcode::IntCast,
                        NodeId::NONE,
                        &[bad],
                        0,
                        ValueType::Rust(f.word),
                    )
                } else {
                    one
                };
                f.straight_jump();
                f.graph.add_node(
                    Opcode::Phi,
                    NodeId::NONE,
                    &[f.control, incoming],
                    0,
                    ValueType::Rust(f.word),
                )
            }
            _ => f.graph.add_node(
                Opcode::DivUnsigned,
                f.control,
                &[one, one],
                0,
                ValueType::Rust(f.word),
            ),
        };
        let old = paired_writes(&mut f, left, value, one, false);
        assert_eq!(f.check(old, true).stores_removed, 1);
        let execution = execute(&f.graph, &[Value::Scalar(0)], false);
        if source == 4 {
            // Entering Region still evaluates the Phi's failing incoming
            // expression before either original or replacement store.
            assert_undefined(&execution);
            assert_eq!(execution.bytes, vec![Some(0x5a); 16]);
        } else {
            assert_eq!(
                execution.result,
                Ok(SemanticOutcome::Return(Value::Scalar(1)))
            );
        }
    }
}

#[test]
fn an_extra_fixed_result_projection_is_not_mistaken_for_an_available_value() {
    let mut f = Fixture::new(0, true);
    let left = f.field(f.address, 0);
    let right = f.field(f.address, 1);
    let load = f.fixed(Opcode::LoadValue, &[right], Some(f.word)).0;
    let duplicate = f.graph.add_node(
        Opcode::Projection,
        NodeId::NONE,
        &[load],
        2,
        ValueType::Rust(f.word),
    );
    let one = f.graph.add_constant(1, f.word);
    let old = paired_writes(&mut f, left, duplicate, one, false);
    assert_eq!(f.check(old, false).stores_removed, 0);
    // The current interpreter records only the canonical result projection.
    // This is an execution-support diagnostic, distinct from semantic UB.
    assert!(matches!(
        execute(&f.graph, &[], false).result,
        Err(InterpretError::Malformed(_))
    ));
}

#[test]
fn operand_evaluation_walks_are_bounded_without_erasing_an_unproved_store() {
    let mut f = Fixture::new(0, true);
    let left = f.field(f.address, 0);
    let one = f.graph.add_constant(1, f.word);
    let mut value = one;
    for _ in 0..40 {
        value = f.graph.add_node(
            Opcode::BitNot,
            NodeId::NONE,
            &[value],
            0,
            ValueType::Rust(f.word),
        );
    }
    let old = paired_writes(&mut f, left, value, one, false);
    let stats = f.check(old, false);
    assert!(stats.budget_bailouts != 0);
}
