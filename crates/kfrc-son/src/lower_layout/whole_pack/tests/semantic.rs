//! The public S→L path crosses an owned enum Phi while retaining independent
//! byte lanes and the original guard until the complete pack is certified.
#[path = "semantic_fixture.rs"]
mod fixture;

use crate::{InterpretError, Interpreter, Opcode, Outcome, SemanticOutcome, Value, ValueType};
use fixture::*;
use kfrc_body::TypeShape;

fn initialized_source(graph: &mut crate::FunctionGraph, bytes: &[u8]) {
    let source = graph
        .ids()
        .find(|&node| {
            graph.op(node) == Opcode::Projection
                && graph.input(node, 0) == graph.start
                && graph.payload(node) == 2
        })
        .unwrap();
    let pointer = graph.rust_type(source).unwrap();
    assert!(matches!(
        graph.oracle.semantic().shape(pointer),
        TypeShape::RawPointer { .. }
    ));
    graph.data_objects.push(kfrc_body::DataObject {
        symbol: "whole_pack_input".into(),
        bytes: bytes.to_vec(),
        align: 1,
        writable: true,
        ..Default::default()
    });
    graph.symbols.push(("whole_pack_input".into(), 0));
    let address = graph.add_node(
        Opcode::SymbolAddress,
        crate::NodeId::NONE,
        &[],
        0,
        ValueType::Rust(pointer),
    );
    graph.replace(source, address);
    graph.resolve_edges();
    crate::verify_graph(graph).unwrap();
}

fn poison_suffix(graph: &mut crate::FunctionGraph, initialized: u64, bytes: u64) {
    let load = graph
        .ids()
        .find(|&node| graph.op(node) == Opcode::LoadValue)
        .unwrap();
    let address = graph.input(load, 1);
    let array = match graph
        .oracle
        .semantic()
        .shape(graph.rust_type(address).unwrap())
    {
        TypeShape::RawPointer { pointee, .. } => *pointee,
        _ => unreachable!(),
    };
    let byte = match graph.oracle.semantic().shape(array) {
        TypeShape::Array { element, .. } => *element,
        _ => unreachable!(),
    };
    let pointer = graph.oracle.intern(
        TypeShape::RawPointer {
            mutability: kfrc_body::Mutability::Mut,
            pointee: byte,
        },
        Some(kfrc_body::SizeAlign { size: 8, align: 8 }),
    );
    let mut control = graph.control(load);
    let mut effect = graph.input(load, 0);
    for offset in initialized..bytes {
        let field = graph.add_node(
            Opcode::FieldAddr,
            crate::NodeId::NONE,
            &[address],
            offset as u32,
            ValueType::Rust(pointer),
        );
        let poison = graph.add_node(
            Opcode::Undefined,
            crate::NodeId::NONE,
            &[],
            0,
            ValueType::Rust(byte),
        );
        let store = graph.add_node(
            Opcode::StoreValue,
            control,
            &[effect, field, poison],
            0,
            ValueType::Tuple(vec![ValueType::Control, ValueType::Effect]),
        );
        control = graph.add_node(
            Opcode::Projection,
            crate::NodeId::NONE,
            &[store],
            0,
            ValueType::Control,
        );
        effect = graph.add_node(
            Opcode::Projection,
            crate::NodeId::NONE,
            &[store],
            1,
            ValueType::Effect,
        );
    }
    graph.set_control(load, control);
    graph.set_input(load, 0, effect);
    crate::verify_graph(graph).unwrap();
}

#[test]
fn owned_enum_phi_loads_combine_through_the_public_layout_pipeline() {
    for bytes in [2, 4, 8] {
        for reverse in [false, true] {
            for overwrite in [false, true] {
                for byte_swap in [false, true] {
                    let options = Options {
                        bytes,
                        reverse,
                        overwrite,
                        byte_swap,
                    };
                    let source = graph("public_guarded_pack", options);
                    assert!(source.ids().any(|node| {
                        source.op(node) == Opcode::Phi
                            && source
                                .rust_type(node)
                                .is_some_and(|ty| source.oracle.semantic().is_enum(ty))
                    }));
                    let (baseline, baseline_stats) = lower(&source, 0);
                    let (candidate, stats) = lower(&source, 128);
                    assert_eq!(baseline_stats.whole_array_transmutes, 0);
                    assert_eq!(stats.whole_array_transmutes, 1);
                    assert_eq!(baseline_stats.whole_packs_combined, 0);
                    assert_eq!(stats.whole_packs_combined, 1, "{}", candidate.dump());
                    assert_eq!(stats.whole_pack_byte_loads_removed, bytes as usize - 1);
                    assert_eq!(stats.whole_pack_payload_phis_removed, bytes as usize - 1);
                    // Budget zero retains the baseline temporary for transmute;
                    // it is a differential of the complete public pipeline.
                    assert!(
                        load_sizes(&baseline)
                            .iter()
                            .filter(|&&size| size == 1)
                            .count()
                            >= bytes as usize
                    );
                    assert_eq!(load_sizes(&candidate), vec![bytes]);
                }
            }
        }
    }
}

#[test]
fn semantic_and_both_layout_forms_keep_errors_effects_and_old_bytes() {
    let contents = [0x98, 0xba, 0xdc, 0xfe, 0x01, 0x7e, 0xff, 0x80];
    for bytes in [2, 4, 8] {
        for overwrite in [false, true] {
            for byte_swap in [false, true] {
                let options = Options {
                    bytes,
                    reverse: true,
                    overwrite,
                    byte_swap,
                };
                let mut source = graph("snapshot_semantics", options);
                initialized_source(&mut source, &contents[..bytes as usize]);
                let forms = [lower(&source, 0).0, lower(&source, 128).0];
                for length in [0, bytes - 1, bytes, bytes + 1, u64::MAX] {
                    let mut semantic_events = vec![];
                    let semantic = Interpreter::new(&source)
                        .run_values(
                            &[Value::Scalar(0), Value::Scalar(length)],
                            &mut |symbol: &str, arguments: &[Value]| {
                                assert_eq!(symbol, "whole_pack_probe");
                                let [Value::Scalar(event)] = arguments else {
                                    panic!("event scalar expected");
                                };
                                semantic_events.push(*event);
                                Ok(Value::Scalar(0))
                            },
                        )
                        .unwrap();
                    let want = if length >= bytes {
                        expected(&contents[..bytes as usize], byte_swap)
                    } else {
                        SENTINEL
                    };
                    assert_eq!(semantic, SemanticOutcome::Return(Value::Scalar(want)));
                    assert_eq!(semantic_events, [1, 2]);
                    for lowered in &forms {
                        let mut events = vec![];
                        let physical = Interpreter::new(lowered)
                            .run(&[0, length], &mut |symbol: &str, arguments: &[u64]| {
                                assert_eq!(symbol, "whole_pack_probe");
                                events.push(arguments[0]);
                                Ok(0)
                            })
                            .unwrap();
                        assert_eq!(physical, Outcome::Return(want), "{}", lowered.dump());
                        assert_eq!(events, semantic_events);
                    }
                }
            }
        }
    }
}

#[test]
fn partial_source_bytes_still_poison_only_an_observed_whole_word() {
    for bytes in [2, 4, 8] {
        for missing_suffix in 1..=bytes {
            let options = Options {
                bytes,
                reverse: false,
                overwrite: false,
                byte_swap: false,
            };
            let mut source = graph("partial_snapshot", options);
            initialized_source(&mut source, &vec![0x80; bytes as usize]);
            poison_suffix(&mut source, bytes - missing_suffix, bytes);
            let forms = [lower(&source, 0).0, lower(&source, 128).0];
            let semantic = Interpreter::new(&source).run_values(
                &[Value::Scalar(0), Value::Scalar(bytes)],
                &mut |_: &str, _: &[Value]| Ok(Value::Scalar(0)),
            );
            assert!(matches!(semantic, Err(InterpretError::PoisonObserved(_))));
            for lowered in &forms {
                assert!(matches!(
                    Interpreter::new(lowered).run(&[0, bytes], &mut |_: &str, _: &[u64]| Ok(0)),
                    Err(InterpretError::PoisonObserved(_))
                ));
                assert_eq!(
                    Interpreter::new(lowered).run(&[0, 0], &mut |_: &str, _: &[u64]| Ok(0)),
                    Ok(Outcome::Return(SENTINEL))
                );
            }
        }
    }
}

#[test]
fn fixed_whole_value_requirement_keeps_the_existing_publishing_contract() {
    let mut source = graph(
        "required_snapshot",
        Options {
            bytes: 8,
            reverse: true,
            overwrite: true,
            byte_swap: false,
        },
    );
    let contents = [0x98, 0xba, 0xdc, 0xfe, 0x01, 0x7e, 0xff, 0x80];
    initialized_source(&mut source, &contents);
    let transmute = source
        .ids()
        .find(|&node| source.op(node) == Opcode::TransmuteValue)
        .unwrap();
    let requirement = source.add_node(
        Opcode::RequireDefined,
        source.control(transmute),
        &[source.input(transmute, 0), source.input(transmute, 1)],
        0,
        ValueType::Tuple(vec![ValueType::Control, ValueType::Effect]),
    );
    let control = source.add_node(
        Opcode::Projection,
        crate::NodeId::NONE,
        &[requirement],
        0,
        ValueType::Control,
    );
    let effect = source.add_node(
        Opcode::Projection,
        crate::NodeId::NONE,
        &[requirement],
        1,
        ValueType::Effect,
    );
    source.set_control(transmute, control);
    source.set_input(transmute, 0, effect);
    crate::verify_graph(&mut source).unwrap();
    let (candidate, stats) = lower(&source, 128);
    assert_eq!(stats.whole_packs_combined, 1);
    assert!(candidate.ids().all(|node| {
        !matches!(
            candidate.op(node),
            Opcode::RequireDefined | Opcode::TransmuteValue
        )
    }));
    for length in [0, 7, 8] {
        let mut semantic_events = Vec::new();
        let semantic = Interpreter::new(&source)
            .run_values(
                &[Value::Scalar(0), Value::Scalar(length)],
                &mut |_: &str, args: &[Value]| {
                    let [Value::Scalar(event)] = args else {
                        panic!("event scalar expected");
                    };
                    semantic_events.push(*event);
                    Ok(Value::Scalar(0))
                },
            )
            .unwrap();
        let want = if length == 8 {
            expected(&contents, false)
        } else {
            SENTINEL
        };
        assert_eq!(semantic, SemanticOutcome::Return(Value::Scalar(want)));
        let mut events = Vec::new();
        assert_eq!(
            Interpreter::new(&candidate).run(&[0, length], &mut |_: &str, args: &[u64]| {
                events.push(args[0]);
                Ok(0)
            }),
            Ok(Outcome::Return(want))
        );
        assert_eq!(events, semantic_events);
    }
    poison_suffix(&mut source, 7, 8);
    let mut events = Vec::new();
    assert!(matches!(
        Interpreter::new(&source).run_values(
            &[Value::Scalar(0), Value::Scalar(8)],
            &mut |_: &str, args: &[Value]| {
                let [Value::Scalar(event)] = args else {
                    panic!("event scalar expected");
                };
                events.push(*event);
                Ok(Value::Scalar(0))
            },
        ),
        Err(InterpretError::PoisonObserved(node)) if node == requirement
    ));
    assert_eq!(events, [1]);
}
