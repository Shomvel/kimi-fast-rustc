use super::super::strength_reduce::{run as reduce, run_with_budget};
use super::*;

fn product(f: &mut Fixture, factor: u64, swapped: bool) -> NodeId {
    let factor = f.graph.add_constant(u128::from(factor), f.word);
    let operands = if swapped {
        [factor, f.iv]
    } else {
        [f.iv, factor]
    };
    f.graph.add_node(
        Opcode::Mul,
        NodeId::NONE,
        &operands,
        0,
        ValueType::Rust(f.word),
    )
}

fn consume(f: &mut Fixture, product: NodeId) {
    f.graph.set_input(f.next_a, 1, product);
}

#[test]
fn every_byte_start_preserves_signed_unsigned_add_sub_recurrences() {
    for signed in [false, true] {
        for subtract in [false, true] {
            for start in 0u8..=255 {
                let end = if subtract {
                    start.wrapping_sub(13)
                } else {
                    start.wrapping_add(13)
                };
                let mut f = Fixture::new(IntegerWidth::W8, signed, start.into(), end.into(), 1);
                if subtract {
                    f.graph.set_opcode(f.next_iv, Opcode::Sub);
                }
                f.graph
                    .set_payload(f.condition, ComparePredicate::NotEqual as u32);
                let p = product(&mut f, 0x93, start % 2 == 0);
                consume(&mut f, p);
                f.verify();
                let before = f.graph.clone();
                let stats = reduce(&mut f.graph);
                assert_eq!(stats.products_reduced, 1, "{signed} {subtract} {start}");
                assert_eq!(stats.recurrences_created, 1);
                f.verify();
                let args = [Value::Scalar(173), Value::Scalar(29)];
                assert_eq!(execute(&before, &args), execute(&f.graph, &args));
            }
        }
    }
}

#[test]
fn dynamic_initial_values_keep_zero_and_many_iterations_at_each_width() {
    for width in [
        IntegerWidth::W8,
        IntegerWidth::W16,
        IntegerWidth::W32,
        IntegerWidth::W64,
    ] {
        let mask = u64::MAX >> (64 - width.bits(64));
        for signed in [false, true] {
            let mut f = Fixture::new(width, signed, 0, 31, 1);
            let x = f.graph.find_projection(f.graph.start, 2).unwrap();
            f.graph.set_input(f.iv, 1, x);
            let p = product(&mut f, 13, signed);
            consume(&mut f, p);
            f.verify();
            let before = f.graph.clone();
            assert_eq!(reduce(&mut f.graph).products_reduced, 1);
            f.verify();
            for initial in [0, 1, 17, 30, 31, 32, mask / 2, mask] {
                let args = [Value::Scalar(initial), Value::Scalar(29)];
                assert_eq!(execute(&before, &args), execute(&f.graph, &args));
            }
        }
    }
}

#[test]
fn arbitrary_modular_steps_and_factors_preserve_boundary_and_random_states() {
    let mut random = 0xabc9_13f8_704e_6555u64;
    for width in [
        IntegerWidth::W8,
        IntegerWidth::W16,
        IntegerWidth::W32,
        IntegerWidth::W64,
    ] {
        let mask = u64::MAX >> (64 - width.bits(64));
        for signed in [false, true] {
            for _ in 0..32 {
                random ^= random << 13;
                random ^= random >> 7;
                random ^= random << 17;
                let start = random & mask;
                let step = random.rotate_left(19) & mask | 1;
                let factor = random.rotate_left(37) & mask | 0x11;
                let end = start.wrapping_add(step.wrapping_mul(13)) & mask;
                let mut f = Fixture::new(width, signed, start, end, step);
                f.graph
                    .set_payload(f.condition, ComparePredicate::NotEqual as u32);
                let p = product(&mut f, factor, signed);
                consume(&mut f, p);
                f.verify();
                let before = f.graph.clone();
                assert_eq!(reduce(&mut f.graph).products_reduced, 1);
                f.verify();
                equivalent(&before, &f.graph, mask);
            }
        }
    }
}

#[test]
fn swapped_predecessors_and_commuted_updates_preserve_simultaneous_phis() {
    let mut f = Fixture::new(IntegerWidth::W32, false, 0, 15, 1);
    let entry = f.graph.input(f.header, 0);
    f.graph.set_inputs(f.header, &[f.latch, entry]);
    for phi in [f.iv, f.a, f.b, f.effect] {
        let args = f.graph.inputs(phi).to_vec();
        f.graph.set_inputs(phi, &[args[0], args[2], args[1]]);
    }
    let update = f.graph.inputs(f.next_iv).to_vec();
    f.graph.set_inputs(f.next_iv, &[update[1], update[0]]);
    let p = product(&mut f, 17, true);
    consume(&mut f, p);
    f.verify();
    let before = f.graph.clone();
    assert_eq!(reduce(&mut f.graph).products_reduced, 1);
    f.verify();
    equivalent(&before, &f.graph, u32::MAX.into());
}

#[test]
fn zero_modular_product_step_becomes_invariant_without_a_new_phi() {
    let mut f = Fixture::new(IntegerWidth::W8, false, 2, 194, 64);
    f.graph
        .set_payload(f.condition, ComparePredicate::NotEqual as u32);
    let p = product(&mut f, 12, false);
    consume(&mut f, p);
    f.verify();
    let before = f.graph.clone();
    let stats = reduce(&mut f.graph);
    assert_eq!(stats.products_reduced, 1);
    assert_eq!(stats.recurrences_created, 0);
    f.verify();
    equivalent(&before, &f.graph, 255);
}

#[test]
fn observations_and_internal_poison_keep_their_original_execution_points() {
    for poison in [false, true] {
        let mut f = Fixture::new(IntegerWidth::W16, false, 0, 15, 1);
        if poison {
            let undefined = f.graph.add_node(
                Opcode::Undefined,
                NodeId::NONE,
                &[],
                0,
                ValueType::Rust(f.word),
            );
            f.graph.set_input(f.iv, 1, undefined);
        }
        let p = product(&mut f, 31, false);
        let observed = f.fixed(Opcode::Opaque, p, true).unwrap();
        consume(&mut f, observed);
        f.verify();
        let before = f.graph.clone();
        assert_eq!(reduce(&mut f.graph).products_reduced, 1);
        f.verify();
        equivalent(&before, &f.graph, 65535);
    }
}

#[test]
fn cheap_factors_arithmetic_promises_and_nonconstant_steps_are_unchanged() {
    for case in 0..11 {
        let mut f = Fixture::new(IntegerWidth::W16, false, 0, 15, 1);
        let factor = match case {
            0..=6 => [0, 1, 2, 3, 5, 8, 9][case],
            _ => 13,
        };
        let p = product(&mut f, factor, false);
        consume(&mut f, p);
        match case {
            7 => f.graph.set_payload(p, ArithmeticFlags::NUW),
            8 => f.graph.set_payload(f.next_iv, ArithmeticFlags::NUW),
            9 => {
                let input = f.graph.find_projection(f.graph.start, 3).unwrap();
                f.graph.set_input(f.next_iv, 1, input);
            }
            10 => {
                let zero = f.graph.add_constant(0, f.word);
                f.graph.set_input(f.next_iv, 1, zero);
            }
            _ => {}
        }
        f.verify();
        let before = f.graph.dump();
        assert_eq!(reduce(&mut f.graph).products_reduced, 0);
        assert_eq!(f.graph.dump(), before);
    }
}

#[test]
fn exit_only_products_and_exhausted_analysis_do_not_add_recurrences() {
    for exhausted in [false, true] {
        let mut f = Fixture::new(IntegerWidth::W64, false, 0, 15, 1);
        let p = product(&mut f, 13, false);
        if exhausted {
            consume(&mut f, p);
        } else {
            let output = f.graph.input(f.ret, 1);
            f.graph.set_input(output, 1, p);
        }
        f.verify();
        let before = f.graph.dump();
        let stats = if exhausted {
            run_with_budget(&mut f.graph, 1)
        } else {
            reduce(&mut f.graph)
        };
        assert_eq!(stats.products_reduced, 0);
        assert_eq!(stats.budget_bailouts, usize::from(exhausted));
        assert_eq!(f.graph.dump(), before);
    }
}

#[test]
fn full_and_seam_pipelines_reduce_products_in_loops_too_large_to_unroll() {
    for seam in [false, true] {
        let mut f = Fixture::new(IntegerWidth::W32, false, 0, 15, 1);
        let p = product(&mut f, 13, false);
        consume(&mut f, p);
        f.verify();
        let before = f.graph.clone();
        let stats = if seam {
            crate::optimize_specialized_seams(&mut f.graph, &[p], &[]).unwrap()
        } else {
            crate::optimize_specialized_graph(&mut f.graph).unwrap()
        };
        assert_eq!(stats.loops_unrolled, 0);
        assert_eq!(stats.loop_products_reduced, 1);
        assert!(f.graph.ids().any(|node| f.graph.op(node) == Opcode::If));
        assert!(!f.graph.ids().any(|node| f.graph.op(node) == Opcode::Mul));
        f.verify();
        equivalent(&before, &f.graph, u32::MAX.into());
    }
}
