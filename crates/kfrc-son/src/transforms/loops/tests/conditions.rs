use super::*;

fn negate(f: &mut Fixture, count: usize) {
    let mut condition = f.condition;
    for _ in 0..count {
        condition = f.graph.add_node(
            Opcode::BitNot,
            NodeId::NONE,
            &[condition],
            0,
            ValueType::Rust(f.boolean),
        );
    }
    f.graph.set_input(f.branch, 0, condition);
}

#[test]
fn negated_comparisons_preserve_polarity_and_zero_trip_behavior() {
    for nots in [1, 2, 7, 8] {
        for reverse_slots in [false, true] {
            for count in [0, 4, 8] {
                let mut f = Fixture::new(IntegerWidth::W32, false, 0, count, 1);
                negate(&mut f, nots);
                if reverse_slots {
                    f.graph.set_payload(f.body, 1);
                    f.graph.set_payload(f.exit, 0);
                }
                if (nots % 2 != 0) != reverse_slots {
                    f.graph
                        .set_payload(f.condition, ComparePredicate::UnsignedGreaterEqual as u32);
                }
                f.verify();
                let before = f.graph.clone();
                let stats = run(&mut f.graph).unwrap();
                assert_eq!(stats.loops_unrolled, 1);
                assert_eq!(stats.iterations_cloned, count as usize);
                f.verify();
                equivalent(&before, &f.graph, u32::MAX.into());
            }
        }
    }
}

#[test]
fn negated_signed_swapped_comparison_keeps_exact_trip_proof() {
    let mut f = Fixture::new(IntegerWidth::W8, true, 2, 254, 1);
    f.graph.set_opcode(f.next_iv, Opcode::Sub);
    f.graph
        .set_payload(f.condition, ComparePredicate::SignedLess as u32);
    let inputs = f.graph.inputs(f.condition).to_vec();
    f.graph.set_inputs(f.condition, &[inputs[1], inputs[0]]);
    negate(&mut f, 1);
    f.graph.set_payload(f.body, 1);
    f.graph.set_payload(f.exit, 0);
    f.verify();
    let before = f.graph.clone();
    let stats = run(&mut f.graph).unwrap();
    assert_eq!(stats.loops_unrolled, 1);
    assert_eq!(stats.iterations_cloned, 4);
    f.verify();
    equivalent(&before, &f.graph, 255);
}

#[test]
fn negation_does_not_prove_large_trip_counts_or_unknown_conditions() {
    let mut f = Fixture::new(IntegerWidth::W8, false, 254, 7, 1);
    f.graph
        .set_payload(f.condition, ComparePredicate::NotEqual as u32);
    negate(&mut f, 1);
    f.graph.set_payload(f.body, 1);
    f.graph.set_payload(f.exit, 0);
    unchanged(f, Limits::default());

    let mut f = Fixture::new(IntegerWidth::W32, false, 0, 4, 1);
    f.graph.set_input(f.condition, 1, f.a);
    negate(&mut f, 1);
    unchanged(f, Limits::default());
}
