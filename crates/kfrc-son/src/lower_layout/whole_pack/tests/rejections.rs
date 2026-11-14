use super::*;
use fixture::project;
use kfrc_body::{ScalarValidRange, ScalarValidity};

fn unchanged(mut f: Fixture) {
    verify_graph(&mut f.graph).unwrap();
    let before = f.graph.dump();
    assert_eq!(run(&mut f.graph, 128), Stats::default(), "{before}");
    assert_eq!(f.graph.dump(), before);
}

#[test]
fn individual_byte_and_pack_intermediate_observers_reject_the_group() {
    for observe in 0..3 {
        let mut f = Fixture::new(8, 2, false, None, 0);
        let source = [f.phis[3], f.casts[3], f.terms[3]][observe];
        let ty = f.graph.rust_type(source).unwrap();
        let zero = f.graph.add_constant(0, ty);
        f.graph.add_node(
            Opcode::BitXor,
            NodeId::NONE,
            &[source, zero],
            0,
            ValueType::Rust(ty),
        );
        unchanged(f);
    }
    let mut f = Fixture::new(8, 1, false, None, 0);
    let load = f.loads[3];
    let value = f.graph.find_projection(load, 2).unwrap();
    f.graph.add_node(
        Opcode::IntCast,
        NodeId::NONE,
        &[value],
        0,
        ValueType::Rust(f.word),
    );
    unchanged(f);
}

#[test]
fn duplicate_missing_out_of_order_and_different_base_addresses_reject() {
    for case in 0..5 {
        let mut f = Fixture::new(8, 1, false, None, 0);
        let address = f.addresses[3];
        match case {
            0 | 1 => {
                let ty = f.graph.rust_type(f.graph.input(address, 1)).unwrap();
                let offset = f.graph.add_constant(if case == 0 { 2 } else { 8 }, ty);
                f.graph.set_input(address, 1, offset);
            }
            2 => {
                let a = f.graph.input(f.addresses[3], 1);
                let b = f.graph.input(f.addresses[4], 1);
                f.graph.set_input(f.addresses[3], 1, b);
                f.graph.set_input(f.addresses[4], 1, a);
            }
            3 => {
                let other = f.graph.add_node(
                    Opcode::Undefined,
                    NodeId::NONE,
                    &[],
                    0,
                    ValueType::Rust(f.pointer),
                );
                f.graph.set_input(address, 0, other);
            }
            4 => {
                let anchor = f.graph.control(f.loads[0]);
                f.graph.set_control(address, anchor);
            }
            _ => unreachable!(),
        }
        unchanged(f);
    }
}

#[test]
fn nonundefined_and_unsynchronized_phi_incoming_slots_reject() {
    for case in 0..3 {
        let mut f = Fixture::new(8, 2, true, None, 0);
        let phi = f.phis[4];
        match case {
            0 => {
                let defined = f.graph.add_constant(0, f.byte);
                f.graph.set_input(phi, 1, defined);
            }
            1 => {
                // A poison-producing expression is not an explicit Undefined
                // certificate even when it would happen to behave the same.
                let poison = f.graph.input(phi, 1);
                let zero = f.graph.add_constant(0, f.byte);
                let derived = f.graph.add_node(
                    Opcode::BitOr,
                    NodeId::NONE,
                    &[poison, zero],
                    0,
                    ValueType::Rust(f.byte),
                );
                f.graph.set_input(phi, 1, derived);
            }
            2 => {
                // An adversarial second merge is not synchronized with the
                // byte load group's predecessor slots.
                let edges = f.graph.inputs(f.region).to_vec();
                let region =
                    f.graph
                        .add_node(Opcode::Region, NodeId::NONE, &edges, 0, ValueType::Control);
                f.graph.set_input(phi, 0, region);
                // This intentionally adversarial graph has a split successor
                // of each Jump; planning must still reject before editing it.
                let before = f.graph.dump();
                assert_eq!(run(&mut f.graph, 128), Stats::default());
                assert_eq!(f.graph.dump(), before);
                continue;
            }
            _ => unreachable!(),
        }
        unchanged(f);
    }
}

#[test]
fn partial_shift_flags_repeated_terms_and_signed_bytes_reject() {
    for case in 0..4 {
        let mut f = Fixture::new(8, 1, false, None, 0);
        match case {
            0 => f
                .graph
                .set_payload(f.terms[3], crate::ArithmeticFlags::AMOUNT_IN_RANGE),
            1 => {
                let shift = f.graph.add_constant(23, f.word);
                f.graph.set_input(f.terms[3], 1, shift);
            }
            2 => {
                let left = f.graph.input(f.pack, 0);
                f.graph.set_input(f.pack, 1, left);
            }
            3 => {
                let signed = f.graph.oracle.intern(
                    TypeShape::Int {
                        width: IntegerWidth::W8,
                        signed: true,
                    },
                    Some(SizeAlign { size: 1, align: 1 }),
                );
                for &phi in &f.phis {
                    for input in [f.graph.input(phi, 1), f.graph.input(phi, 2)] {
                        f.graph.set_value_type(input, ValueType::Rust(signed));
                        if f.graph.op(input) == Opcode::Projection {
                            let load = f.graph.input(input, 0);
                            f.graph.set_value_type(
                                load,
                                ValueType::Tuple(vec![
                                    ValueType::Control,
                                    ValueType::Effect,
                                    ValueType::Rust(signed),
                                ]),
                            );
                        }
                    }
                    f.graph.set_value_type(phi, ValueType::Rust(signed));
                }
            }
            _ => unreachable!(),
        }
        unchanged(f);
    }
}

#[test]
fn extra_control_effect_and_projection_observers_reject() {
    for case in 0..4 {
        let mut f = Fixture::new(8, 1, true, None, 0);
        let load = f.loads[3];
        let control = f.graph.find_projection(load, 0).unwrap();
        let effect = f.graph.find_projection(load, 1).unwrap();
        match case {
            0 => {
                f.graph
                    .add_node(Opcode::Unreachable, control, &[], 0, ValueType::Control);
            }
            1 => {
                // An additional dead value use of a token is enough to prevent
                // retiring its producer; no expression may retain a dead edge.
                f.graph.add_node(
                    Opcode::Phi,
                    NodeId::NONE,
                    &[f.region, effect, f.input_effect],
                    0,
                    ValueType::Effect,
                );
            }
            2 => {
                project(&mut f.graph, load, 2, ValueType::Rust(f.byte));
            }
            3 => {
                let last = *f.loads.last().unwrap();
                let effect = f.graph.find_projection(last, 1).unwrap();
                f.graph.add_node(
                    Opcode::Phi,
                    NodeId::NONE,
                    &[f.region, effect, f.input_effect],
                    0,
                    ValueType::Effect,
                );
            }
            _ => unreachable!(),
        }
        // These extra edges model a stale/abandoned graph builder as well as
        // a real effect fork. The pass must reject without relying on cleanup.
        let before = f.graph.dump();
        assert_eq!(run(&mut f.graph, 128), Stats::default());
        assert_eq!(f.graph.dump(), before);
    }
}

#[test]
fn an_interleaved_store_prevents_load_coalescing() {
    let mut f = Fixture::new(8, 1, false, None, 0);
    let next = f.loads[4];
    let control = f.graph.control(next);
    let effect = f.graph.input(next, 0);
    let zero = f.graph.add_constant(0, f.byte);
    let store = f.graph.add_node(
        Opcode::Store,
        control,
        &[effect, f.base, zero],
        0,
        ValueType::Tuple(vec![ValueType::Control, ValueType::Effect]),
    );
    let control = project(&mut f.graph, store, 0, ValueType::Control);
    let effect = project(&mut f.graph, store, 1, ValueType::Effect);
    f.graph.set_control(next, control);
    f.graph.set_input(next, 0, effect);
    unchanged(f);
}

#[test]
fn differing_partitions_and_restricted_or_vector_words_reject() {
    let mut f = Fixture::new(8, 1, false, None, 0);
    f.graph
        .memory_partitions
        .push(crate::MemoryPartition::Unknown);
    f.graph.set_payload(f.loads[4], 1);
    unchanged(f);
    for case in 0..3 {
        let mut f = Fixture::new(8, 1, false, None, 0);
        match case {
            0 => f.graph.oracle.record_scalar_validity(
                f.word,
                ScalarValidity::Scalar(ScalarValidRange::new(64, 1, u128::from(u64::MAX)).unwrap()),
            ),
            1 => f.graph.oracle.record_vector(
                f.word,
                kfrc_body::VectorLayout {
                    element: f.byte,
                    lanes: 8,
                },
            ),
            2 => {
                f.graph.oracle.unions.insert(f.word);
            }
            _ => unreachable!(),
        }
        let before = f.graph.dump();
        assert_eq!(run(&mut f.graph, 128), Stats::default());
        assert_eq!(f.graph.dump(), before);
    }
}
