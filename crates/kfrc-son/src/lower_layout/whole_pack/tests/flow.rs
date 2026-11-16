use super::*;

#[test]
fn forward_diamond_inside_an_outer_loop_is_not_a_loop_carried_pack() {
    let mut f = Fixture::new(8, 1, true, None, 0);
    let source_branch = f.graph.input(f.graph.control(f.loads[0]), 0);
    let before = f.graph.control(source_branch);
    let entry = f
        .graph
        .add_node(Opcode::Jump, before, &[], 0, ValueType::Control);
    let inactive_ret = f
        .graph
        .ids()
        .find(|&node| f.graph.op(node) == Opcode::Return && node != f.ret)
        .unwrap();
    let back = f.graph.add_node(
        Opcode::Jump,
        f.graph.control(inactive_ret),
        &[],
        0,
        ValueType::Control,
    );
    let header = f.graph.add_node(
        Opcode::Region,
        NodeId::NONE,
        &[entry, back],
        0,
        ValueType::Control,
    );
    let effect = f.graph.add_node(
        Opcode::Phi,
        NodeId::NONE,
        &[header, f.input_effect, f.effect_phi],
        0,
        ValueType::Effect,
    );
    f.graph.set_control(source_branch, header);
    f.graph.set_input(f.loads[0], 0, effect);
    f.graph.set_input(f.effect_phi, 3 - f.slot, effect);
    f.graph.remove_node(inactive_ret);
    verify_graph(&mut f.graph).unwrap();
    let before = f.graph.clone();
    assert_eq!(run(&mut f.graph, 128).groups, 1);
    verify_graph(&mut f.graph).unwrap();
    // The active execution exits after one forward diamond. The inactive path
    // is an intentional divergent outer loop and must never become a read.
    same_execution(&before, &f.graph, true);
}

#[test]
fn repeated_forward_diamonds_capture_each_iterations_bytes_before_the_next_store() {
    let mut f = Fixture::new(8, 2, true, None, 1);
    let source_branch = f.graph.input(f.graph.control(f.loads[0]), 0);
    let entry = f.graph.add_node(
        Opcode::Jump,
        f.graph.control(source_branch),
        &[],
        0,
        ValueType::Control,
    );
    let header = f.graph.add_node(
        Opcode::Region,
        NodeId::NONE,
        &[entry],
        0,
        ValueType::Control,
    );
    let zero = f.graph.add_constant(0, f.word);
    let one = f.graph.add_constant(1, f.word);
    let limit = f.graph.add_constant(7, f.word);
    let mut phi = |ty, initial| {
        f.graph.add_node(
            Opcode::Phi,
            NodeId::NONE,
            &[header, initial, initial],
            0,
            ty,
        )
    };
    let counter = phi(ValueType::Rust(f.word), zero);
    let sum = phi(ValueType::Rust(f.word), zero);
    let input_effect = phi(ValueType::Effect, f.input_effect);
    f.graph.set_control(source_branch, header);
    f.graph.set_input(f.loads[0], 0, input_effect);
    f.graph.set_input(f.effect_phi, 3 - f.slot, input_effect);
    let next_counter = f.graph.add_node(
        Opcode::Add,
        NodeId::NONE,
        &[counter, one],
        0,
        ValueType::Rust(f.word),
    );
    let next_sum = f.graph.add_node(
        Opcode::Add,
        NodeId::NONE,
        &[sum, f.pack],
        0,
        ValueType::Rust(f.word),
    );
    let byte = f.graph.add_node(
        Opcode::IntCast,
        NodeId::NONE,
        &[next_counter],
        0,
        ValueType::Rust(f.byte),
    );
    let store = f.graph.add_node(
        Opcode::Store,
        f.graph.control(f.ret),
        &[f.effect_phi, f.base, byte],
        0,
        ValueType::Tuple(vec![ValueType::Control, ValueType::Effect]),
    );
    let control = fixture::project(&mut f.graph, store, 0, ValueType::Control);
    let effect = fixture::project(&mut f.graph, store, 1, ValueType::Effect);
    let boolean = f.graph.rust_type(f.active).unwrap();
    let again = f.graph.add_node(
        Opcode::Compare,
        NodeId::NONE,
        &[next_counter, limit],
        crate::ComparePredicate::UnsignedLess as u32,
        ValueType::Rust(boolean),
    );
    let branch = f.graph.add_node(
        Opcode::If,
        control,
        &[again],
        0,
        ValueType::Tuple(vec![ValueType::Control; 2]),
    );
    let again = fixture::project(&mut f.graph, branch, 0, ValueType::Control);
    let done = fixture::project(&mut f.graph, branch, 1, ValueType::Control);
    let back = f
        .graph
        .add_node(Opcode::Jump, again, &[], 0, ValueType::Control);
    f.graph.set_inputs(header, &[entry, back]);
    f.graph.set_input(counter, 2, next_counter);
    f.graph.set_input(sum, 2, next_sum);
    f.graph.set_input(input_effect, 2, effect);
    f.graph.set_control(f.ret, done);
    f.graph.set_input(f.ret, 0, effect);
    let returned = f.graph.input(f.ret, 1);
    f.graph.set_input(returned, 0, next_sum);
    verify_graph(&mut f.graph).unwrap();
    let before = f.graph.clone();
    assert_eq!(run(&mut f.graph, 128).groups, 1);
    verify_graph(&mut f.graph).unwrap();
    let expected = (1..7).fold(f.expected(), |sum, first_byte| {
        sum.wrapping_add((f.expected() & !0xff) | first_byte)
    });
    assert_eq!(evaluate(&f.graph, true), Ok(Outcome::Return(expected)));
    assert_eq!(
        evaluate(&f.graph, false),
        Ok(Outcome::Return(fixture::SENTINEL))
    );
    same_execution(&before, &f.graph, true);
    same_execution(&before, &f.graph, false);
}

#[test]
fn a_header_dominating_its_backedge_has_no_forward_join_certificate() {
    let mut f = Fixture::new(4, 1, false, None, 0);
    let source_branch = f.graph.input(f.graph.control(f.loads[0]), 0);
    let entry = f.graph.add_node(
        Opcode::Jump,
        f.graph.control(source_branch),
        &[],
        0,
        ValueType::Control,
    );
    let back = f.graph.input(f.region, f.slot - 1);
    let old_other = f.graph.input(f.region, 2 - f.slot);
    f.graph.set_input(f.region, 2 - f.slot, entry);
    f.graph.set_control(source_branch, f.region);
    f.graph.remove_node(old_other);
    let uses = Uses::new(&f.graph);
    let mut fuel = Fuel {
        left: FUEL,
        exhausted: false,
    };
    assert!(index::forward_join(&f.graph, &uses, f.region, &mut fuel).is_none());
    assert_eq!(
        f.graph.control(back),
        f.graph
            .find_projection(*f.loads.last().unwrap(), 0)
            .unwrap()
    );
    let before = f.graph.dump();
    assert_eq!(run(&mut f.graph, 128), Stats::default());
    assert_eq!(f.graph.dump(), before);
}

#[test]
fn unreachable_and_duplicate_predecessors_have_no_forward_certificate() {
    for duplicate in [false, true] {
        let mut f = Fixture::new(2, 2, false, None, 0);
        if duplicate {
            let edge = f.graph.input(f.region, 0);
            f.graph.set_input(f.region, 1, edge);
        } else {
            let detached = f.graph.add_node(
                Opcode::Projection,
                NodeId::NONE,
                &[f.graph.start],
                0,
                ValueType::Control,
            );
            let edge = f
                .graph
                .add_node(Opcode::Jump, detached, &[], 0, ValueType::Control);
            f.graph.set_input(f.region, 0, edge);
            // A duplicate Start control is reachable, so explicitly detach the
            // predecessor under a Region with no path from the real Start.
            let isolated =
                f.graph
                    .add_node(Opcode::Region, NodeId::NONE, &[edge], 0, ValueType::Control);
            f.graph.set_control(edge, isolated);
        }
        let uses = Uses::new(&f.graph);
        let mut fuel = Fuel {
            left: FUEL,
            exhausted: false,
        };
        assert!(index::forward_join(&f.graph, &uses, f.region, &mut fuel).is_none());
    }
}

#[test]
fn shared_partial_observers_cannot_form_overlapping_plans() {
    let mut f = Fixture::new(2, 1, false, None, 0);
    // Clone the pure pack into another word observer. Its byte Phis now have
    // two casts each, so neither cone can claim shared partial observations.
    let mut terms = Vec::new();
    for (index, &phi) in f.phis.iter().enumerate() {
        let cast = f.graph.add_node(
            Opcode::IntCast,
            NodeId::NONE,
            &[phi],
            0,
            ValueType::Rust(f.word),
        );
        terms.push(if index == 0 {
            cast
        } else {
            let eight = f.graph.add_constant(8, f.word);
            f.graph.add_node(
                Opcode::ShiftLeft,
                NodeId::NONE,
                &[cast, eight],
                0,
                ValueType::Rust(f.word),
            )
        });
    }
    f.graph.add_node(
        Opcode::BitOr,
        NodeId::NONE,
        &terms,
        0,
        ValueType::Rust(f.word),
    );
    verify_graph(&mut f.graph).unwrap();
    let before = f.graph.dump();
    assert_eq!(run(&mut f.graph, 128), Stats::default());
    assert_eq!(f.graph.dump(), before);
}

#[test]
fn sequential_independent_groups_publish_atomically_even_with_cross_plan_effects() {
    let mut f = Fixture::new(8, 2, true, None, 1);
    f.append_second_group();
    verify_graph(&mut f.graph).unwrap();
    let before = f.graph.clone();
    let mut successful = None;
    for budget in 0..FUEL {
        let mut graph = before.clone();
        let stats = run_with_fuel(&mut graph, 128, budget);
        if stats.groups != 0 {
            assert_eq!(stats.groups, 2);
            assert_eq!(stats.byte_loads_removed, 14);
            assert_eq!(stats.payload_phis_removed, 14);
            verify_graph(&mut graph).unwrap();
            assert_eq!(
                evaluate(&graph, true),
                Ok(Outcome::Return(f.expected().wrapping_mul(2)))
            );
            same_execution(&before, &graph, true);
            same_execution(&before, &graph, false);
            successful = Some(budget);
            break;
        }
        assert!(stats.budget_exhausted);
        assert_eq!(graph.dump(), before.dump());
    }
    assert!(successful.is_some_and(|budget| budget < 20_000));
}
