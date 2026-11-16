use super::*;

#[test]
fn every_byte_start_has_exact_modular_four_step_add_and_sub() {
    for signed in [false, true] {
        for op in [Opcode::Add, Opcode::Sub] {
            for start in 0u8..=255 {
                let end = if op == Opcode::Add {
                    start.wrapping_add(4)
                } else {
                    start.wrapping_sub(4)
                };
                let mut f = Fixture::new(
                    IntegerWidth::W8,
                    signed,
                    u64::from(start),
                    u64::from(end),
                    1,
                );
                f.graph.set_opcode(f.next_iv, op);
                f.graph
                    .set_payload(f.condition, ComparePredicate::NotEqual as u32);
                f.verify();
                let before = f.graph.clone();
                let stats = run(&mut f.graph).unwrap();
                assert_eq!(stats.loops_unrolled, 1, "{signed} {op:?} {start}");
                assert_eq!(stats.iterations_cloned, 4);
                f.verify();
                let args = [Value::Scalar(173), Value::Scalar(29)];
                assert_eq!(execute(&before, &args), execute(&f.graph, &args));
            }
        }
    }
}

#[test]
fn modular_recurrences_keep_width_and_branch_predicate_semantics() {
    for width in [IntegerWidth::W8, IntegerWidth::W32, IntegerWidth::W64] {
        let mask = u64::MAX >> (64 - width.bits(64));
        for signed in [false, true] {
            for step in [1, 2, 127, mask, mask - 1] {
                for count in [0, 1, 7, 8] {
                    let start = mask - 1;
                    let end = start.wrapping_add(step.wrapping_mul(count)) & mask;
                    let mut f = Fixture::new(width, signed, start, end, step);
                    f.graph
                        .set_payload(f.condition, ComparePredicate::NotEqual as u32);
                    f.verify();
                    let before = f.graph.clone();
                    let stats = run(&mut f.graph).unwrap();
                    assert_eq!(stats.loops_unrolled, 1);
                    assert_eq!(stats.iterations_cloned, count as usize);
                    f.verify();
                    equivalent(&before, &f.graph, mask);
                }
            }
        }
    }
}

#[test]
fn modular_nontermination_large_trip_counts_and_poison_flags_stay_untouched() {
    for (start, end, step) in [(0, 1, 2), (254, 7, 1), (0, 1, 0)] {
        let mut f = Fixture::new(IntegerWidth::W8, false, start, end, step);
        f.graph
            .set_payload(f.condition, ComparePredicate::NotEqual as u32);
        unchanged(f, Limits::default());
    }
    for (signed, start, end, flag) in [
        (false, 254, 2, ArithmeticFlags::NUW),
        (true, 126, 130, ArithmeticFlags::NSW),
    ] {
        let mut f = Fixture::new(IntegerWidth::W8, signed, start, end, 1);
        f.graph
            .set_payload(f.condition, ComparePredicate::NotEqual as u32);
        f.graph.set_payload(f.next_iv, flag);
        unchanged(f, Limits::default());
    }
}
