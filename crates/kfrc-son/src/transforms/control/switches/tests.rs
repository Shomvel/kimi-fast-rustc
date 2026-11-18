use super::*;
use crate::interpret::Outcome;
use crate::{CallTarget, GraphState, Interpreter, verify_graph};
use kfrc_body::{FunctionAbi, IntegerWidth, Mutability, PassMode, RecordedTypes, SizeAlign, TyId};

fn projection(graph: &mut FunctionGraph, source: NodeId, slot: u32, ty: ValueType) -> NodeId {
    graph.add_node(Opcode::Projection, NodeId::NONE, &[source], slot, ty)
}

struct Fixture {
    graph: FunctionGraph,
    word: TyId,
    input: NodeId,
    effect: NodeId,
    branches: Vec<NodeId>,
    no: Vec<NodeId>,
    wrappers: Vec<NodeId>,
}

fn chain(width: IntegerWidth, signed: bool, cases: &[u128], wrapped: bool) -> Fixture {
    let mut types = RecordedTypes::new();
    let bytes = u64::from(width.bits(64) / 8);
    let word = types.intern(
        TypeShape::Int { width, signed },
        Some(SizeAlign {
            size: bytes,
            align: bytes,
        }),
    );
    let boolean = types.intern(TypeShape::Bool, Some(SizeAlign { size: 1, align: 1 }));
    let mut graph = FunctionGraph::new(
        "integer_chain",
        FunctionAbi {
            args: vec![PassMode::Direct],
            ret: PassMode::Direct,
            c_variadic: false,
        },
        types,
    );
    graph.level = GraphLevel::Layout;
    let start = graph.add_node(
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
    graph.start = start;
    let mut control = projection(&mut graph, start, 0, ValueType::Control);
    let effect = projection(&mut graph, start, 1, ValueType::Effect);
    let input = projection(&mut graph, start, 2, ValueType::Rust(word));
    let mut branches = Vec::new();
    let mut no = Vec::new();
    let mut wrappers = Vec::new();
    let mut jumps = Vec::new();
    let mut results = Vec::new();
    for (index, &case) in cases.iter().enumerate() {
        let constant = graph.add_constant(case, word);
        let operands = if index % 2 == 0 {
            [input, constant]
        } else {
            [constant, input]
        };
        let equal = index % 3 != 0;
        let mut condition = graph.add_node(
            Opcode::Compare,
            NodeId::NONE,
            &operands,
            if equal {
                ComparePredicate::Equal
            } else {
                ComparePredicate::NotEqual
            } as u32,
            ValueType::Rust(boolean),
        );
        let inverted = index % 4 == 0;
        if inverted {
            condition = graph.add_node(
                Opcode::BitNot,
                NodeId::NONE,
                &[condition],
                0,
                ValueType::Rust(boolean),
            );
        }
        let branch = graph.add_node(
            Opcode::If,
            control,
            &[condition],
            0,
            ValueType::Tuple(vec![ValueType::Control; 2]),
        );
        branches.push(branch);
        let truth = projection(&mut graph, branch, 0, ValueType::Control);
        let falsehood = projection(&mut graph, branch, 1, ValueType::Control);
        let (yes, next) = if equal ^ inverted {
            (truth, falsehood)
        } else {
            (falsehood, truth)
        };
        no.push(next);
        jumps.push(graph.add_node(Opcode::Jump, yes, &[], 0, ValueType::Control));
        results.push(graph.add_constant(index as u128 + 1, word));
        control = next;
        if wrapped && index + 1 != cases.len() {
            let jump = graph.add_node(Opcode::Jump, control, &[], 0, ValueType::Control);
            control = graph.add_node(Opcode::Region, NodeId::NONE, &[jump], 0, ValueType::Control);
            wrappers.push(control);
        }
    }
    jumps.push(graph.add_node(Opcode::Jump, control, &[], 0, ValueType::Control));
    results.push(graph.add_constant(cases.len() as u128 + 1, word));
    let region = graph.add_node(Opcode::Region, NodeId::NONE, &jumps, 0, ValueType::Control);
    let mut inputs = vec![region];
    inputs.extend(results);
    let result = graph.add_node(Opcode::Phi, NodeId::NONE, &inputs, 0, ValueType::Rust(word));
    graph.add_node(
        Opcode::Return,
        region,
        &[effect, result],
        0,
        ValueType::Control,
    );
    graph.state = GraphState::Sealed;
    Fixture {
        graph,
        word,
        input,
        effect,
        branches,
        no,
        wrappers,
    }
}

fn execute(graph: &FunctionGraph, input: u64) -> Outcome {
    Interpreter::new(graph)
        .run(&[input], &mut |_: &str, _: &[u64]| {
            panic!("unexpected call")
        })
        .unwrap()
}

fn share_matches(fixture: &mut Fixture, count: usize) -> NodeId {
    let graph = &mut fixture.graph;
    let phi = graph
        .ids()
        .find(|&node| graph.op(node) == Opcode::Phi)
        .unwrap();
    let result = graph.input(phi, 1);
    for slot in 2..=count {
        graph.set_input(phi, slot, result);
    }
    graph.input(phi, 0)
}

#[test]
fn compact_membership_matches_every_byte_including_wrapped_indices() {
    for (cases, remaining_branches) in [
        (&[1, 2, 7][..], 2),
        (&[4, 3, 5][..], 1),
        (&[255, 0, 254][..], 1),
        (&[242, 248, 252, 255][..], 2),
    ] {
        for signed in [false, true] {
            let mut fixture = chain(IntegerWidth::W8, signed, cases, true);
            share_matches(&mut fixture, cases.len());
            verify_graph(&mut fixture.graph).unwrap();
            let original = fixture.graph.clone();
            assert_eq!(
                run_with_membership(&mut fixture.graph, true).switches_created,
                0
            );
            verify_graph(&mut fixture.graph).unwrap();
            assert_eq!(
                fixture
                    .graph
                    .ids()
                    .filter(|&n| fixture.graph.op(n) == Opcode::If)
                    .count(),
                remaining_branches
            );
            assert!(crate::schedule::Schedule::new(&fixture.graph).is_ok());
            for input in 0..=255 {
                assert_eq!(
                    execute(&fixture.graph, input),
                    execute(&original, input),
                    "{cases:?} {input}"
                );
            }
        }
    }
}

#[test]
fn compact_membership_does_not_truncate_the_guard_or_discard_the_residual_test() {
    let mut fixture = chain(IntegerWidth::W64, true, &[1, 31, 7, 1u128 << 32], false);
    share_matches(&mut fixture, 3);
    verify_graph(&mut fixture.graph).unwrap();
    let original = fixture.graph.clone();
    let stats = run_with_membership(&mut fixture.graph, true);
    assert_eq!(stats.bit_memberships, 1);
    assert_eq!(stats.switches_created, 0);
    verify_graph(&mut fixture.graph).unwrap();
    for input in [
        0,
        1,
        7,
        31,
        32,
        33,
        63,
        1u64 << 32,
        (1u64 << 32) + 1,
        u64::MAX,
    ] {
        assert_eq!(execute(&fixture.graph, input), execute(&original, input));
    }
}

#[test]
fn membership_keeps_distinct_values_and_anchored_observations() {
    let cases = [1, 2, 7];
    let mut fixture = chain(IntegerWidth::W8, false, &cases, true);
    assert_eq!(
        run_with_membership(&mut fixture.graph, true).bit_memberships,
        0
    );
    assert_eq!(
        fixture
            .graph
            .ids()
            .filter(|&n| fixture.graph.op(n) == Opcode::If)
            .count(),
        3
    );

    let region = share_matches(&mut fixture, cases.len());
    let graph = &mut fixture.graph;
    let hit_jump = graph.input(region, 1);
    let hit = graph.control(hit_jump);
    let one = graph.add_constant(1, fixture.word);
    graph.add_node(
        Opcode::DivUnsigned,
        hit,
        &[fixture.input, one],
        0,
        ValueType::Rust(fixture.word),
    );
    verify_graph(graph).unwrap();
    assert_eq!(run_with_membership(graph, true).bit_memberships, 0);
    assert_eq!(
        graph.ids().filter(|&n| graph.op(n) == Opcode::If).count(),
        3
    );
    verify_graph(graph).unwrap();
}

#[test]
fn switch_matches_every_byte_and_preserves_shared_phi_slots() {
    for signed in [false, true] {
        for wrapped in [false, true] {
            let cases = [255, 0, 128, 3, 65, 17, 42, 2];
            let mut fixture = chain(IntegerWidth::W8, signed, &cases, wrapped);
            verify_graph(&mut fixture.graph).unwrap();
            let original = fixture.graph.clone();
            let stats = run(&mut fixture.graph);
            assert_eq!(stats.switches_created, 1);
            assert_eq!(stats.cases_merged, cases.len());
            verify_graph(&mut fixture.graph).unwrap();
            let schedule = crate::schedule::Schedule::new(&fixture.graph).unwrap();
            assert!(
                schedule
                    .blocks
                    .iter()
                    .any(|block| block.successors.len() == cases.len() + 1)
            );
            for input in 0..=255 {
                let expected = cases
                    .iter()
                    .position(|case| *case == u128::from(input))
                    .unwrap_or(cases.len())
                    + 1;
                assert_eq!(
                    execute(&fixture.graph, input),
                    Outcome::Return(expected as u64)
                );
                assert_eq!(execute(&fixture.graph, input), execute(&original, input));
            }
        }
    }
}

#[test]
fn wide_and_nonzero_based_cases_keep_all_discriminant_bits() {
    for (width, cases, samples) in [
        (
            IntegerWidth::W32,
            vec![10, 11, 12, 13, u128::from(u32::MAX)],
            vec![0, 9, 10, 13, 14, u64::from(u32::MAX)],
        ),
        (
            IntegerWidth::W64,
            vec![1, 2, 3, 4, 1u128 << 32, u128::from(u64::MAX)],
            vec![0, 1, 3, 5, 1u64 << 32, (1u64 << 32) + 1, u64::MAX],
        ),
    ] {
        let mut fixture = chain(width, false, &cases, true);
        verify_graph(&mut fixture.graph).unwrap();
        let original = fixture.graph.clone();
        assert_eq!(run(&mut fixture.graph).switches_created, 1);
        verify_graph(&mut fixture.graph).unwrap();
        for input in samples {
            assert_eq!(execute(&fixture.graph, input), execute(&original, input));
        }
    }
}

#[test]
fn bounded_chunks_preserve_nested_dispatch_and_residual_default() {
    let cases: Vec<u128> = (0..260).collect();
    let mut fixture = chain(IntegerWidth::W16, false, &cases, true);
    verify_graph(&mut fixture.graph).unwrap();
    let original = fixture.graph.clone();
    let stats = run(&mut fixture.graph);
    assert_eq!(stats.switches_created, 3);
    assert_eq!(stats.cases_merged, 260);
    verify_graph(&mut fixture.graph).unwrap();
    for input in [0, 1, 127, 128, 129, 255, 256, 259, 260, 65535] {
        assert_eq!(execute(&fixture.graph, input), execute(&original, input));
    }
}

#[test]
fn layout_pipeline_forms_switch_after_scalar_cleanup() {
    let mut fixture = chain(IntegerWidth::W64, false, &[5, 6, 7, 8, 9, 10], false);
    fixture.graph.level = GraphLevel::Specialized;
    verify_graph(&mut fixture.graph).unwrap();
    let stats = crate::lower_layout(&mut fixture.graph).unwrap();
    assert_eq!(stats.integer_switches_created, 1);
    verify_graph(&mut fixture.graph).unwrap();
    assert!(
        fixture
            .graph
            .ids()
            .any(|node| fixture.graph.op(node) == Opcode::Switch)
    );
}

#[test]
fn fixed_events_and_phi_consumers_stop_transparent_collection() {
    for barrier in [
        Opcode::Opaque,
        Opcode::Load,
        Opcode::Call,
        Opcode::Store,
        Opcode::DivUnsigned,
    ] {
        let mut fixture = chain(IntegerWidth::W64, false, &[1, 2, 3, 4], true);
        // Two tests on each side of the event: neither residual chain reaches
        // the minimum. The matcher must not speculate any fixed operation.
        let control = fixture.wrappers[1];
        let effect = fixture.effect;
        let word = fixture.word;
        let input = fixture.input;
        let graph = &mut fixture.graph;
        if barrier == Opcode::DivUnsigned {
            let one = graph.add_constant(1, word);
            graph.add_node(barrier, control, &[input, one], 0, ValueType::Rust(word));
        } else {
            let mut payload = 0;
            let inputs = match barrier {
                Opcode::Call => {
                    payload = graph.add_callee(CallTarget {
                        symbol: "switch_barrier".into(),
                        abi: FunctionAbi {
                            args: vec![PassMode::Direct],
                            ret: PassMode::Direct,
                            c_variadic: false,
                        },
                    });
                    vec![effect, input]
                }
                Opcode::Load | Opcode::Store => {
                    let pointer = graph.oracle.intern(
                        TypeShape::RawPointer {
                            mutability: Mutability::Mut,
                            pointee: word,
                        },
                        Some(SizeAlign { size: 8, align: 8 }),
                    );
                    let slot = graph.stack_slots.len() as u32;
                    graph.stack_slots.push(SizeAlign { size: 8, align: 8 });
                    let address = graph.add_node(
                        Opcode::StackSlot,
                        NodeId::NONE,
                        &[],
                        slot,
                        ValueType::Rust(pointer),
                    );
                    if barrier == Opcode::Load {
                        vec![effect, address]
                    } else {
                        vec![effect, address, input]
                    }
                }
                _ => vec![effect, input],
            };
            let mut results = vec![ValueType::Control, ValueType::Effect];
            if barrier != Opcode::Store {
                results.push(ValueType::Rust(word));
            }
            let event = graph.add_node(
                barrier,
                control,
                &inputs,
                payload,
                ValueType::Tuple(results),
            );
            let after = projection(graph, event, 0, ValueType::Control);
            graph.set_control(fixture.branches[2], after);
        }
        verify_graph(&mut fixture.graph).unwrap();
        assert_eq!(run(&mut fixture.graph).switches_created, 0, "{barrier:?}");
        verify_graph(&mut fixture.graph).unwrap();
    }
    let mut fixture = chain(IntegerWidth::W64, false, &[1, 2, 3, 4], true);
    let region = fixture.wrappers[1];
    fixture.graph.add_node(
        Opcode::Phi,
        NodeId::NONE,
        &[region, fixture.input],
        0,
        ValueType::Rust(fixture.word),
    );
    verify_graph(&mut fixture.graph).unwrap();
    assert_eq!(run(&mut fixture.graph).switches_created, 0);
}

#[test]
fn extra_control_uses_changed_values_and_duplicate_cases_are_not_absorbed() {
    let mut fixture = chain(IntegerWidth::W64, false, &[1, 2, 3, 4], false);
    fixture
        .graph
        .add_node(Opcode::Jump, fixture.no[1], &[], 0, ValueType::Control);
    assert_eq!(run(&mut fixture.graph).switches_created, 0);
    let mut fixture = chain(IntegerWidth::W64, false, &[1, 2, 3, 4], false);
    let condition = fixture.graph.input(fixture.branches[2], 0);
    let other = fixture.graph.add_constant(99, fixture.word);
    fixture.graph.set_input(condition, 0, other);
    assert_eq!(run(&mut fixture.graph).switches_created, 0);
    let mut fixture = chain(IntegerWidth::W64, false, &[1, 2, 1, 4], false);
    assert_eq!(run(&mut fixture.graph).switches_created, 0);
}

#[test]
fn switch_observes_poison_at_the_dispatch() {
    let mut fixture = chain(IntegerWidth::W8, false, &[1, 2, 3, 4], false);
    let poison = fixture.graph.add_node(
        Opcode::Undefined,
        NodeId::NONE,
        &[],
        0,
        ValueType::Rust(fixture.word),
    );
    fixture.graph.replace(fixture.input, poison);
    assert_eq!(run(&mut fixture.graph).switches_created, 1);
    verify_graph(&mut fixture.graph).unwrap();
    assert!(matches!(
        Interpreter::new(&fixture.graph).run(&[0], &mut |_: &str, _: &[u64]| Ok(0)),
        Err(crate::InterpretError::PoisonObserved(_))
    ));
}

#[test]
fn verifier_rejects_bad_switch_contracts() {
    let mut fixture = chain(IntegerWidth::W8, false, &[1, 2, 3, 4], false);
    assert_eq!(run(&mut fixture.graph).switches_created, 1);
    let baseline = fixture.graph;
    for defect in 0..11 {
        let mut graph = baseline.clone();
        let switch = graph
            .ids()
            .find(|node| graph.op(*node) == Opcode::Switch)
            .unwrap();
        let payload = graph.payload(switch) as usize;
        let edge = graph.find_projection(switch, 0).unwrap();
        match defect {
            0 => graph.set_payload(switch, u32::MAX),
            1 => graph.integer_switches[payload][1] = graph.integer_switches[payload][0],
            2 => graph.integer_switches[payload][0] = 256,
            3 => graph.set_value_type(switch, ValueType::Tuple(vec![ValueType::Control; 2])),
            4 => graph.set_value_type(edge, ValueType::Effect),
            5 => graph.set_payload(edge, 9),
            6 => graph.level = GraphLevel::Specialized,
            7 => {
                let effect = graph
                    .ids()
                    .find(|node| graph.value_type(*node) == &ValueType::Effect)
                    .unwrap();
                graph.set_input(switch, 0, effect);
            }
            8 => {
                graph.add_node(
                    Opcode::Projection,
                    NodeId::NONE,
                    &[switch],
                    0,
                    ValueType::Control,
                );
            }
            9 => {
                let input = graph.input(switch, 0);
                let pointer = graph.oracle.intern(
                    TypeShape::RawPointer {
                        mutability: Mutability::Mut,
                        pointee: fixture.word,
                    },
                    Some(SizeAlign { size: 8, align: 8 }),
                );
                graph.set_value_type(input, ValueType::Rust(pointer));
                let mut results = match graph.value_type(graph.start) {
                    ValueType::Tuple(results) => results.clone(),
                    _ => unreachable!(),
                };
                results[2] = ValueType::Rust(pointer);
                graph.set_value_type(graph.start, ValueType::Tuple(results));
            }
            10 => {
                graph.add_node(
                    Opcode::Switch,
                    graph.control(switch),
                    &[graph.input(switch, 0)],
                    payload as u32,
                    graph.value_type(switch).clone(),
                );
            }
            _ => unreachable!(),
        }
        assert!(
            verify_graph(&mut graph).is_err(),
            "accepted defect {defect}"
        );
    }
}
