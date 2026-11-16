use super::*;
use crate::CallTarget;
use kfrc_body::{FunctionAbi, PassMode, ScalarValidRange, ScalarValidity};

fn source(f: &mut Fixture) -> Value {
    let array = f.graph.oracle.intern(
        TypeShape::Array {
            element: f.byte,
            length: f.bytes as u64,
        },
        Some(SizeAlign {
            size: f.bytes as u64,
            align: 1,
        }),
    );
    Value {
        ty: array,
        parts: Parts::Fields(
            f.phis
                .iter()
                .map(|&node| Value {
                    ty: f.byte,
                    parts: Parts::Scalar(node),
                })
                .collect(),
        ),
    }
}

#[test]
fn narrow_bridge_checks_the_entire_representation_before_mutating() {
    for case in 0..11 {
        let mut f = Fixture::new(8, 1, false, None, 0);
        let mut value = source(&mut f);
        match case {
            0 => {
                value.parts = Parts::OwnedStorage(f.base);
            }
            1 => {
                value.parts = Parts::Repeat {
                    element: Box::new(Value {
                        ty: f.byte,
                        parts: Parts::Scalar(f.phis[0]),
                    }),
                    count: 8,
                    overrides: BTreeMap::new(),
                };
            }
            2 | 3 | 4 => {
                let Parts::Fields(fields) = &mut value.parts else {
                    unreachable!();
                };
                match case {
                    2 => fields[7].parts = Parts::Omitted,
                    3 => fields[7].ty = f.word,
                    4 => {
                        fields.pop();
                    }
                    _ => unreachable!(),
                }
            }
            5 => f.graph.oracle.record_scalar_validity(
                f.word,
                ScalarValidity::Scalar(ScalarValidRange::new(64, 1, u128::from(u64::MAX)).unwrap()),
            ),
            6 => f.graph.oracle.record_scalar_validity(
                f.byte,
                ScalarValidity::Scalar(ScalarValidRange::new(8, 1, 255).unwrap()),
            ),
            7 => {
                f.graph.oracle.unions.insert(value.ty);
            }
            8 => f.graph.oracle.record_vector(
                value.ty,
                kfrc_body::VectorLayout {
                    element: f.byte,
                    lanes: 8,
                },
            ),
            9 => {
                value.ty = f.graph.oracle.intern(
                    TypeShape::Array {
                        element: f.byte,
                        length: 7,
                    },
                    Some(SizeAlign { size: 8, align: 1 }),
                );
            }
            10 => {
                let signed = f.graph.oracle.intern(
                    TypeShape::Int {
                        width: IntegerWidth::W8,
                        signed: true,
                    },
                    Some(SizeAlign { size: 1, align: 1 }),
                );
                value.ty = f.graph.oracle.intern(
                    TypeShape::Array {
                        element: signed,
                        length: 8,
                    },
                    Some(SizeAlign { size: 8, align: 1 }),
                );
            }
            _ => unreachable!(),
        }
        let before = f.graph.dump();
        let len = f.graph.len();
        assert!(transmute::pack(&mut f.graph, &value, f.word, 128).is_none());
        assert_eq!(f.graph.dump(), before, "case {case}");
        assert_eq!(f.graph.len(), len, "case {case}");
    }
}

#[test]
fn bridge_keeps_partial_byte_observers_and_their_independent_poison() {
    for bytes in [2, 4, 8] {
        for missing in 0..bytes {
            let mut f = Fixture::new(bytes, 2, false, Some(missing), 1);
            let value = source(&mut f);
            let packed = transmute::pack(&mut f.graph, &value, f.word, 128).unwrap();
            let Parts::Scalar(packed) = packed.parts else {
                unreachable!();
            };
            f.graph.replace(f.pack, packed);
            // A fixed observer of a different byte keeps it independently
            // available even when the whole pack contains a poison byte.
            let index = (missing + 1) % bytes;
            let observed = f.phis[index];
            let observed = f.graph.add_node(
                Opcode::IntCast,
                NodeId::NONE,
                &[observed],
                0,
                ValueType::Rust(f.word),
            );
            let callee = f.graph.add_callee(CallTarget {
                symbol: "observe_byte".into(),
                abi: FunctionAbi {
                    args: vec![PassMode::Direct],
                    ret: PassMode::Direct,
                    c_variadic: false,
                },
            });
            let call = f.graph.add_node(
                Opcode::Call,
                f.graph.control(f.ret),
                &[f.graph.input(f.ret, 0), observed],
                callee,
                ValueType::Tuple(vec![
                    ValueType::Control,
                    ValueType::Effect,
                    ValueType::Rust(f.word),
                ]),
            );
            let control = fixture::project(&mut f.graph, call, 0, ValueType::Control);
            let effect = fixture::project(&mut f.graph, call, 1, ValueType::Effect);
            f.graph.set_control(f.ret, control);
            f.graph.set_input(f.ret, 0, effect);
            f.graph.compact();
            verify_graph(&mut f.graph).unwrap();
            let before = f.graph.clone();
            assert_eq!(run(&mut f.graph, 128), Stats::default());
            verify_graph(&mut f.graph).unwrap();
            let execute = |graph: &FunctionGraph| {
                let mut events = Vec::new();
                let result = Interpreter::new(graph).run(&[1], &mut |name: &str, args: &[u64]| {
                    assert_eq!(name, "observe_byte");
                    events.push(args[0]);
                    Ok(0)
                });
                assert!(matches!(result, Err(InterpretError::PoisonObserved(_))));
                events
            };
            let expected = (f.expected() >> (index * 8)) & 0xff;
            assert_eq!(execute(&before), [expected]);
            assert_eq!(execute(&f.graph), [expected]);
            same_execution(&before, &f.graph, false);
        }
    }
}
