use super::*;
use crate::{CallTarget, GraphState, InterpretError, Interpreter, Outcome, verify_graph};
use kfrc_body::{DataObject, FunctionAbi, Mutability, PassMode, RecordedTypes, SizeAlign, TyId};

mod fixture;
use fixture::Fixture;

#[test]
fn interleaved_lengths_preserve_first_match_and_exact_call_sequence() {
    let mut fixture = Fixture::new(&[5, 2, 5, 1, 2, 4, 1, 4, 5], false, false);
    verify_graph(&mut fixture.graph).unwrap();
    let original = fixture.graph.clone();
    let stats = transform(&mut fixture.graph, MAX_WORK);
    assert_eq!(
        (
            stats.ladders_grouped,
            stats.cells_grouped,
            stats.length_buckets
        ),
        (1, 9, 4)
    );
    assert_eq!(stats.nodes_added, 11);
    verify_graph(&mut fixture.graph).unwrap();
    for length in [0, 1, 2, 3, 4, 5, 32, u64::MAX] {
        for outcomes in 0..512 {
            let before = Fixture::execute(&original, length, outcomes);
            let after = Fixture::execute(&fixture.graph, length, outcomes);
            assert_eq!(after, before, "length={length} outcomes={outcomes}");
        }
    }
    let before = Fixture::execute(&original, 5, 0b1_0000_0101);
    assert_eq!(before.0, Outcome::Return(1));
    assert_eq!(before.1.len(), 1, "later successful duplicate must not run");
}

#[test]
fn retained_byte_loads_and_shared_success_phi_positions_match() {
    let mut fixture = Fixture::new(&[4, 1, 2, 1, 4, 2, 1, 3], true, true);
    verify_graph(&mut fixture.graph).unwrap();
    let original = fixture.graph.clone();
    let stats = transform(&mut fixture.graph, MAX_WORK);
    assert_eq!(stats.cells_grouped, 8);
    verify_graph(&mut fixture.graph).unwrap();
    for length in [0, 1, 2, 3, 4, 8] {
        for outcomes in 0..256 {
            assert_eq!(
                Fixture::execute(&fixture.graph, length, outcomes),
                Fixture::execute(&original, length, outcomes)
            );
        }
    }
    // The raw byte paths remain separate reads, and each original Call stays
    // present; grouping must not introduce a purity-based CSE/deletion.
    for opcode in [Opcode::Load, Opcode::Call] {
        assert_eq!(
            fixture
                .graph
                .ids()
                .filter(|node| fixture.graph.op(*node) == opcode)
                .count(),
            original
                .ids()
                .filter(|node| original.op(*node) == opcode)
                .count()
        );
    }
}

#[test]
fn entry_available_fixed_result_stays_before_dispatch() {
    let mut fixture = Fixture::new(&[5, 2, 5, 2], false, false);
    let first = fixture.events[0];
    let incoming = fixture.graph.input(first, 0);
    let pointer = fixture.graph.input(first, 1);
    let pointer_type = fixture.graph.value_type(pointer).clone();
    let event = fixture.graph.add_node(
        Opcode::Opaque,
        fixture.entry,
        &[incoming, pointer],
        0,
        ValueType::Tuple(vec![
            ValueType::Control,
            ValueType::Effect,
            pointer_type.clone(),
        ]),
    );
    let after = fixture.graph.add_node(
        Opcode::Projection,
        NodeId::NONE,
        &[event],
        0,
        ValueType::Control,
    );
    let effect = fixture.graph.add_node(
        Opcode::Projection,
        NodeId::NONE,
        &[event],
        1,
        ValueType::Effect,
    );
    let pointer =
        fixture
            .graph
            .add_node(Opcode::Projection, NodeId::NONE, &[event], 2, pointer_type);
    fixture.graph.set_control(fixture.guards[0], after);
    fixture.graph.set_input(first, 0, effect);
    let phi = fixture
        .graph
        .ids()
        .find(|node| {
            fixture.graph.op(*node) == Opcode::Phi
                && fixture.graph.input(*node, 0) == fixture.failures[0]
        })
        .unwrap();
    fixture.graph.set_input(phi, 1, effect);
    for &call in &fixture.events {
        fixture.graph.set_input(call, 1, pointer);
    }
    verify_graph(&mut fixture.graph).unwrap();
    let original = fixture.graph.clone();
    assert_eq!(transform(&mut fixture.graph, MAX_WORK).cells_grouped, 4);
    verify_graph(&mut fixture.graph).unwrap();
    for length in 0..=6 {
        for outcomes in 0..16 {
            assert_eq!(
                Fixture::execute(&fixture.graph, length, outcomes),
                Fixture::execute(&original, length, outcomes)
            );
        }
    }
}

#[test]
fn authentication_external_uses_and_dependent_addresses_decline_without_edits() {
    for defect in 0..7 {
        let mut fixture = Fixture::new(&[5, 2, 5, 2], false, false);
        let first = fixture.events[0];
        match defect {
            0 => {
                let target = fixture.graph.add_callee(CallTarget {
                    symbol: "bcmp".into(),
                    abi: RuntimeComparison::Bcmp.abi(),
                });
                fixture.graph.set_payload(first, target);
            }
            1 => {
                let target = fixture.graph.payload(first) as usize;
                fixture.graph.callees[target].abi.c_variadic = true;
            }
            2 => {
                let result = fixture.graph.find_projection(first, 2).unwrap();
                let ret = fixture.success_returns[0];
                let widened = fixture.graph.add_node(
                    Opcode::IntCast,
                    NodeId::NONE,
                    &[result],
                    0,
                    ValueType::Rust(fixture.word),
                );
                fixture.graph.set_input(ret, 1, widened);
            }
            3 => {
                let effect = fixture.graph.find_projection(first, 1).unwrap();
                fixture
                    .graph
                    .set_input(fixture.success_returns[1], 0, effect);
            }
            4 => {
                let source = fixture.graph.input(fixture.events[1], 1);
                let region = fixture.failures[0];
                let phi = fixture.graph.add_node(
                    Opcode::Phi,
                    NodeId::NONE,
                    &[region, source, source],
                    0,
                    fixture.graph.value_type(source).clone(),
                );
                fixture.graph.set_input(fixture.events[1], 1, phi);
            }
            5 => {
                let value = fixture.graph.add_constant(99, fixture.word);
                let tail = *fixture.failures.last().unwrap();
                fixture.graph.add_node(
                    Opcode::Phi,
                    NodeId::NONE,
                    &[tail, value, value],
                    0,
                    ValueType::Rust(fixture.word),
                );
            }
            6 => {
                let constant = fixture.graph.add_constant(7, fixture.word);
                let region = fixture.success_regions[0];
                fixture.graph.add_node(
                    Opcode::Phi,
                    NodeId::NONE,
                    &[region, constant],
                    0,
                    ValueType::Rust(fixture.word),
                );
            }
            _ => unreachable!(),
        }
        fixture.decline();
    }
}

#[test]
fn fixed_events_extra_entries_and_cycles_are_not_transparent() {
    for defect in 0..3 {
        let mut fixture = Fixture::new(&[3, 2, 3, 2], false, false);
        let first = fixture.events[0];
        match defect {
            0 => {
                let control = fixture.graph.control(first);
                let effect = fixture.graph.input(first, 0);
                let target = fixture.graph.add_callee(CallTarget {
                    symbol: "observe".into(),
                    abi: FunctionAbi {
                        args: vec![],
                        ret: PassMode::Direct,
                        c_variadic: false,
                    },
                });
                fixture.graph.add_node(
                    Opcode::Call,
                    control,
                    &[effect],
                    target,
                    ValueType::Tuple(vec![
                        ValueType::Control,
                        ValueType::Effect,
                        ValueType::Rust(fixture.word),
                    ]),
                );
            }
            1 => {
                let failure = fixture.failures[0];
                let original = fixture.graph.input(failure, 0);
                let extra =
                    fixture
                        .graph
                        .add_node(Opcode::Jump, fixture.entry, &[], 0, ValueType::Control);
                fixture
                    .graph
                    .set_inputs(failure, &[original, extra, fixture.graph.input(failure, 1)]);
            }
            2 => {
                let condition = fixture.graph.input(fixture.guards[0], 0);
                fixture.graph.set_opcode(condition, Opcode::BitNot);
                fixture.graph.set_inputs(condition, &[condition]);
            }
            _ => unreachable!(),
        }
        fixture.decline();
    }
}

#[test]
fn exhausted_work_abandons_the_complete_plan_without_graph_edits() {
    let mut fixture = Fixture::new(&[5, 2, 5, 2, 4, 1, 4, 1], false, true);
    let before = fixture.graph.dump();
    let callees = fixture.graph.callees.clone();
    for work in [0, 1, 32, 128, 512] {
        let stats = transform(&mut fixture.graph, work);
        assert_eq!(stats.ladders_grouped, 0);
        assert!(stats.budget_exhausted);
        assert_eq!(fixture.graph.dump(), before);
        assert_eq!(fixture.graph.callees, callees);
        assert!(fixture.graph.integer_switches.is_empty());
    }
    let mut probe = fixture.graph.clone();
    let completed = transform(&mut probe, MAX_WORK);
    assert_eq!(completed.cells_grouped, 8);
    let stats = transform(&mut fixture.graph, completed.node_visits - 1);
    assert!(
        stats.budget_exhausted,
        "final reservation must fail before its first edit"
    );
    assert_eq!(fixture.graph.dump(), before);
    let stats = transform(&mut fixture.graph, completed.node_visits);
    assert_eq!(stats.cells_grouped, 8);
}
