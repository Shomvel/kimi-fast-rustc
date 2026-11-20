//! Publish first, then thread scalar decisions hidden behind S observations.
#[path = "../src/lower_layout/whole_pack/tests/semantic_fixture.rs"]
#[allow(dead_code)]
mod fixture;
#[path = "support/refinement.rs"]
mod refinement;

use kfrc_body::{FunctionAbi, PassMode, TyId, TypeShape};
use kfrc_son::{
    CallTarget, ComparePredicate, FunctionGraph, GraphLevel, InterpretError, Interpreter,
    LayoutOptions, LayoutStats, NodeId, Opcode, Outcome, SemanticOutcome, Value, ValueType,
    lower_layout, lower_layout_with_options, optimize_specialized_graph, verify_graph,
};

const CONTENTS: [u8; 8] = [0x98, 0xba, 0xdc, 0xfe, 0x01, 0x7e, 0xff, 0x80];

fn count(graph: &FunctionGraph, op: Opcode) -> usize {
    graph.ids().filter(|&node| graph.op(node) == op).count()
}

fn project(graph: &mut FunctionGraph, source: NodeId, slot: u32, ty: ValueType) -> NodeId {
    graph.add_node(Opcode::Projection, NodeId::NONE, &[source], slot, ty)
}

fn fixed(
    graph: &mut FunctionGraph,
    op: Opcode,
    control: NodeId,
    inputs: &[NodeId],
    payload: u32,
    result: Option<TyId>,
) -> [NodeId; 2] {
    let mut outputs = vec![ValueType::Control, ValueType::Effect];
    outputs.extend(result.map(ValueType::Rust));
    let node = graph.add_node(op, control, inputs, payload, ValueType::Tuple(outputs));
    [
        project(graph, node, 0, ValueType::Control),
        project(graph, node, 1, ValueType::Effect),
    ]
}

fn source(reverse: bool, overwrite: bool, poison: bool) -> FunctionGraph {
    let mut graph = fixture::graph(
        "layout_threading",
        fixture::Options {
            bytes: 8,
            reverse,
            overwrite,
            byte_swap: false,
        },
    );
    let load = graph
        .ids()
        .find(|&n| graph.op(n) == Opcode::LoadValue)
        .unwrap();
    let pointer = graph.rust_type(graph.input(load, 1)).unwrap();
    graph.data_objects.push(kfrc_body::DataObject {
        symbol: "layout_threading_input".into(),
        bytes: CONTENTS.to_vec(),
        align: 1,
        writable: true,
        ..Default::default()
    });
    graph.symbols.push(("layout_threading_input".into(), 0));
    let address = graph.add_node(
        Opcode::SymbolAddress,
        NodeId::NONE,
        &[],
        0,
        ValueType::Rust(pointer),
    );
    let argument = graph.input(load, 1);
    graph.replace(argument, address);
    graph.resolve_edges();
    let switch = graph
        .ids()
        .find(|&n| graph.op(n) == Opcode::VariantSwitch)
        .unwrap();
    let region = graph.control(switch);
    let joined = graph.input(switch, 0);
    let effect = graph
        .ids()
        .find(|&n| graph.op(n) == Opcode::Phi && graph.value_type(n) == &ValueType::Effect)
        .unwrap();
    let word = graph
        .rust_type(
            graph
                .ids()
                .find(|&n| {
                    graph.op(n) == Opcode::Projection
                        && graph.input(n, 0) == graph.start
                        && graph.payload(n) == 3
                })
                .unwrap(),
        )
        .unwrap();
    // Both incoming effects have visible work. The success snapshot still
    // ends immediately before its Jump, so P can combine the exact byte reads.
    for slot in 0..2 {
        let success = graph.payload(graph.input(joined, slot + 1)) == 0;
        let target = if success {
            load
        } else {
            graph.input(region, slot)
        };
        let event = graph.add_constant(if success { 3 } else { 4 }, word);
        let callee = graph.add_callee(CallTarget {
            symbol: "whole_pack_probe".into(),
            abi: FunctionAbi {
                args: vec![PassMode::Direct],
                ret: PassMode::Direct,
                c_variadic: false,
            },
        });
        let incoming = if success {
            graph.input(load, 0)
        } else {
            graph.input(effect, slot + 1)
        };
        let before_control = graph.control(target);
        let [control, after] = fixed(
            &mut graph,
            Opcode::Call,
            before_control,
            &[incoming, event],
            callee,
            Some(word),
        );
        graph.set_control(target, control);
        if success {
            graph.set_input(load, 0, after);
        } else {
            graph.set_input(effect, slot + 1, after);
        }
    }
    if poison {
        let TypeShape::RawPointer { pointee: array, .. } = *graph.oracle.semantic().shape(pointer)
        else {
            unreachable!()
        };
        let TypeShape::Array { element, .. } = *graph.oracle.semantic().shape(array) else {
            unreachable!()
        };
        let byte_pointer = graph.oracle.intern(
            TypeShape::RawPointer {
                mutability: kfrc_body::Mutability::Mut,
                pointee: element,
            },
            Some(kfrc_body::SizeAlign { size: 8, align: 8 }),
        );
        let first = graph.add_node(
            Opcode::FieldAddr,
            NodeId::NONE,
            &[address],
            0,
            ValueType::Rust(byte_pointer),
        );
        let poison = graph.add_node(
            Opcode::Undefined,
            NodeId::NONE,
            &[],
            0,
            ValueType::Rust(element),
        );
        let control = graph.control(load);
        let incoming = graph.input(load, 0);
        let [control, after] = fixed(
            &mut graph,
            Opcode::StoreValue,
            control,
            &[incoming, first, poison],
            0,
            None,
        );
        graph.set_control(load, control);
        graph.set_input(load, 0, after);
    }
    // The whole enum's payload came from LoadValue, so S cannot prove this
    // requirement redundant. Publishing erases it without moving the reads.
    let [control, after] = fixed(
        &mut graph,
        Opcode::RequireDefined,
        region,
        &[effect, joined],
        0,
        None,
    );
    let users: Vec<_> = graph
        .ids()
        .filter(|&n| graph.op(n) != Opcode::RequireDefined)
        .collect();
    for node in users {
        let inputs = graph.inputs(node).to_vec();
        for (slot, input) in inputs.into_iter().enumerate() {
            if input == effect {
                graph.set_input(node, slot, after);
            }
        }
    }
    graph.set_control(switch, control);
    verify_graph(&mut graph).unwrap();
    graph
}

fn lower(source: &FunctionGraph) -> FunctionGraph {
    let mut graph = source.clone();
    lower_layout(&mut graph).unwrap();
    verify_graph(&mut graph).unwrap();
    graph
}

fn compare(
    source: &FunctionGraph,
    target: &FunctionGraph,
    length: u64,
) -> (Result<u64, InterpretError>, Vec<u64>) {
    let mut events = Vec::new();
    let before = Interpreter::new(source)
        .run_values(
            &[Value::Scalar(0), Value::Scalar(length)],
            &mut |name: &str, args: &[Value]| {
                assert_eq!(name, "whole_pack_probe");
                let [Value::Scalar(event)] = args else {
                    panic!("scalar event expected")
                };
                events.push(*event);
                Ok(Value::Scalar(0))
            },
        )
        .map(|outcome| match outcome {
            SemanticOutcome::Return(Value::Scalar(bits)) => bits,
            other => panic!("scalar return expected: {other:?}"),
        });
    let mut after_events = Vec::new();
    let after = Interpreter::new(target)
        .run(&[0, length], &mut |name: &str, args: &[u64]| {
            assert_eq!(name, "whole_pack_probe");
            after_events.push(args[0]);
            Ok(0)
        })
        .map(|outcome| match outcome {
            Outcome::Return(bits) => bits,
            other => panic!("scalar return expected: {other:?}"),
        });
    refinement::assert_release_refines(&before, &after, |a, b| a == b);
    if before.is_ok() {
        assert_eq!(events, after_events);
    }
    (before, events)
}

#[test]
fn publishing_releases_enum_dispatch_while_preserving_snapshots_payloads_and_effects() {
    for reverse in [false, true] {
        for overwrite in [false, true] {
            for poison in [false, true] {
                let mut source = source(reverse, overwrite, poison);
                optimize_specialized_graph(&mut source).unwrap();
                // Either the merged check stays and Layout publishing threads
                // the dispatch, or (KFRC_HOIST_MERGE_CHECKS, default on) the
                // check runs at the end of each predecessor and the dispatch
                // is already threaded before Layout. Both must lower to the
                // same single residual test with identical behavior.
                let shape = (
                    count(&source, Opcode::RequireDefined),
                    count(&source, Opcode::VariantSwitch),
                );
                assert!(matches!(shape, (1, 1) | (2, 0)), "{shape:?}\n{}", source.dump());
                let target = lower(&source);
                assert_eq!(count(&target, Opcode::If), 1, "{}", target.dump());
                assert_eq!(count(&target, Opcode::Call), 5);
                if !poison {
                    assert_eq!(fixture::load_sizes(&target), [8]);
                }
                for length in [0, 7, 8, 9, u64::MAX] {
                    let (result, events) = compare(&source, &target, length);
                    if length < 8 {
                        assert_eq!(result, Ok(fixture::SENTINEL));
                        assert_eq!(events, [1, 4, 2]);
                    } else if poison {
                        assert!(matches!(result, Err(InterpretError::PoisonObserved(_))));
                        assert_eq!(events, [1, 3]);
                    } else {
                        assert_eq!(result, Ok(fixture::expected(&CONTENTS, false)));
                        assert_eq!(events, [1, 3, 2]);
                    }
                }
            }
        }
    }
}

#[test]
fn an_unused_division_at_the_merge_prevents_bypassing_its_fixed_execution() {
    let mut source = source(false, true, false);
    let requirement = source
        .ids()
        .find(|&n| source.op(n) == Opcode::RequireDefined)
        .unwrap();
    let length = source
        .ids()
        .find(|&n| {
            source.op(n) == Opcode::Projection
                && source.input(n, 0) == source.start
                && source.payload(n) == 3
        })
        .unwrap();
    source.add_node(
        Opcode::DivUnsigned,
        source.control(requirement),
        &[length, length],
        0,
        source.value_type(length).clone(),
    );
    optimize_specialized_graph(&mut source).unwrap();
    let target = lower(&source);
    assert_eq!(count(&target, Opcode::DivUnsigned), 1);
    assert_eq!(count(&target, Opcode::If), 2);
    for length in [0, 1, 7, 8, 9] {
        let (result, _) = compare(&source, &target, length);
        assert_eq!(result.is_ok(), length != 0);
    }
}

#[test]
fn a_real_loop_header_keeps_its_phi_slots_and_reexecutes_the_requirement() {
    let mut source = source(true, false, false);
    let switch = source
        .ids()
        .find(|&n| source.op(n) == Opcode::VariantSwitch)
        .unwrap();
    let joined = source.input(switch, 0);
    let region = source.input(joined, 0);
    let effect = source
        .ids()
        .find(|&n| source.op(n) == Opcode::Phi && source.value_type(n) == &ValueType::Effect)
        .unwrap();
    let inactive = source.find_projection(switch, 1).unwrap();
    let call = source
        .ids()
        .find(|&n| source.op(n) == Opcode::Call && source.control(n) == inactive)
        .unwrap();
    let continuation = source.find_projection(call, 0).unwrap();
    let after = source.find_projection(call, 1).unwrap();
    let ret = source
        .ids()
        .find(|&n| source.op(n) == Opcode::Return && source.control(n) == continuation)
        .unwrap();
    source.remove_node(ret);
    let back = source.add_node(Opcode::Jump, continuation, &[], 0, ValueType::Control);
    let result_ty = source.rust_type(joined).unwrap();
    let array = source
        .oracle
        .semantic()
        .field_type(result_ty, Some(0), 0)
        .unwrap();
    let TypeShape::Array { element, .. } = *source.oracle.semantic().shape(array) else {
        unreachable!()
    };
    let byte = source.add_constant(0x51, element);
    let payload = source.add_node(
        Opcode::Aggregate,
        NodeId::NONE,
        &[byte; 8],
        0,
        ValueType::Rust(array),
    );
    let next = source.add_node(
        Opcode::MakeVariant,
        NodeId::NONE,
        &[payload],
        0,
        ValueType::Rust(result_ty),
    );
    for (node, incoming) in [(region, back), (joined, next), (effect, after)] {
        let mut inputs = source.inputs(node).to_vec();
        inputs.push(incoming);
        source.set_inputs(node, &inputs);
    }
    verify_graph(&mut source).unwrap();
    let target = lower(&source);
    assert_eq!(count(&target, Opcode::If), 2);
    for length in [0, 7, 8, 9] {
        let (result, events) = compare(&source, &target, length);
        if length < 8 {
            assert_eq!(result, Ok(u64::from_le_bytes([0x51; 8])));
            assert_eq!(events, [1, 4, 2, 2]);
        } else {
            assert_eq!(result, Ok(fixture::expected(&CONTENTS, false)));
            assert_eq!(events, [1, 3, 2]);
        }
    }
}
