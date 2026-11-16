use super::*;
use kfrc_body::Mutability;
use kfrc_son::{ComparePredicate, SemanticOutcome, fold_scalar_ranges};

#[test]
fn only_validated_non_zst_slice_references_have_a_byte_extent_bound() {
    for element_size in [None, Some(0), Some(1), Some(4)] {
        for raw in [false, true] {
            for mutability in [Mutability::Not, Mutability::Mut] {
                for undefined in [false, true] {
                    let mut types = RecordedTypes::new();
                    let element = types.intern(
                        TypeShape::Other {
                            name: "Element".into(),
                        },
                        element_size.map(|size| SizeAlign { size, align: 1 }),
                    );
                    let pointee = types.intern(TypeShape::Slice { element }, None);
                    let metadata = types.intern(
                        TypeShape::Int {
                            width: IntegerWidth::Pointer,
                            signed: false,
                        },
                        Some(SizeAlign { size: 8, align: 8 }),
                    );
                    let data = types.intern(
                        TypeShape::RawPointer {
                            mutability,
                            pointee: element,
                        },
                        Some(SizeAlign { size: 8, align: 8 }),
                    );
                    let pointer = types.intern(
                        if raw {
                            TypeShape::RawPointer {
                                mutability,
                                pointee,
                            }
                        } else {
                            TypeShape::Reference {
                                mutability,
                                pointee,
                            }
                        },
                        Some(SizeAlign { size: 16, align: 8 }),
                    );
                    types.record_aggregate(
                        pointer,
                        AggregateLayout {
                            fields: vec![
                                FieldLayout {
                                    ty: data,
                                    offset: 0,
                                },
                                FieldLayout {
                                    ty: metadata,
                                    offset: 8,
                                },
                            ],
                            variants: vec![],
                        },
                    );
                    let mut graph = identity(types, pointer, pointer);
                    let ret = graph
                        .ids()
                        .find(|&n| graph.op(n) == Opcode::Return)
                        .unwrap();
                    let source = if undefined {
                        graph.add_node(
                            Opcode::Undefined,
                            NodeId::NONE,
                            &[],
                            0,
                            ValueType::Rust(pointer),
                        )
                    } else {
                        graph.input(ret, 1)
                    };
                    let length = graph.add_node(
                        Opcode::Field,
                        NodeId::NONE,
                        &[source],
                        1,
                        ValueType::Rust(metadata),
                    );
                    graph.set_input(ret, 1, length);
                    let size = element_size.filter(|&size| size > 0);
                    let bound = (i64::MAX as u64) / size.unwrap_or(1);
                    compare_return(
                        &mut graph,
                        ComparePredicate::UnsignedLessEqual,
                        u128::from(bound),
                    );
                    let before = graph.clone();
                    let expected = !raw && !undefined && size.is_some();
                    assert_eq!(
                        fold_scalar_ranges(&mut graph).comparisons_folded,
                        usize::from(expected)
                    );
                    let lengths: &[u64] = if raw || element_size == Some(0) {
                        &[0, 1, 7, u64::MAX]
                    } else {
                        &[0, 1, 7]
                    };
                    for &length in lengths {
                        let input = Value::WidePointer {
                            data: Box::new(Value::Scalar(0x1000)),
                            metadata: Box::new(Value::Scalar(length)),
                        };
                        let original = run(&before, input.clone());
                        let actual = run(&graph, input);
                        if undefined {
                            assert!(matches!(original, Err(InterpretError::PoisonObserved(_))));
                            assert!(matches!(actual, Err(InterpretError::PoisonObserved(_))));
                        } else {
                            assert_eq!(actual, original);
                            assert_eq!(
                                actual,
                                Ok(SemanticOutcome::Return(Value::Scalar(u64::from(
                                    length <= bound
                                ))))
                            );
                        }
                    }
                }
            }
        }
    }
}
