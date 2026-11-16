use super::*;

#[test]
fn reads_observations_calls_and_unreachable_on_one_arm_stop_the_entire_proof() {
    for barrier in [
        Opcode::LoadValue,
        Opcode::RequireDefined,
        Opcode::Opaque,
        Opcode::Call,
        Opcode::Unreachable,
    ] {
        let mut f = Fixture::new(1, true);
        let left = f.field(f.address, 0);
        let one = f.graph.add_constant(1, f.word);
        let two = f.graph.add_constant(2, f.word);
        let old = f.store(left, one);
        let states = f.branch(f.conditions[0]);
        f.enter(states[0]);
        f.store(left, two);
        f.load_return(left);
        f.enter(states[1]);
        let mut loaded = None;
        match barrier {
            Opcode::LoadValue => {
                loaded = f.fixed(barrier, &[left], Some(f.word)).1;
            }
            Opcode::RequireDefined => {
                let poison = f.poison(f.word);
                f.fixed(barrier, &[poison], None);
            }
            Opcode::Opaque => {
                f.fixed(barrier, &[one], Some(f.word));
            }
            Opcode::Call => {
                f.graph.add_callee(CallTarget {
                    symbol: "observe_memory".into(),
                    abi: FunctionAbi {
                        args: vec![PassMode::Direct],
                        ret: PassMode::Direct,
                        c_variadic: false,
                    },
                });
                f.fixed(barrier, &[left], Some(f.word));
            }
            Opcode::Unreachable => {
                f.graph
                    .add_node(barrier, f.control, &[], 0, ValueType::Control);
            }
            _ => unreachable!(),
        }
        if barrier != Opcode::Unreachable {
            f.store(left, two);
            if let Some(loaded) = loaded {
                f.ret(loaded);
            } else {
                f.load_return(left);
            }
        }
        assert_eq!(f.check(old, false).stores_removed, 0, "{barrier:?}");
        if barrier == Opcode::Call {
            let execution = execute(&f.graph, &[Value::Scalar(0)], false);
            let mut bytes = vec![Some(0x5a); 16];
            bytes[..8].copy_from_slice(&1u64.to_le_bytes().map(Some));
            assert_eq!(execution.calls, [bytes]);
        }
    }
}

#[test]
fn anchored_division_and_remainder_are_seen_on_every_control_token() {
    for op in [
        Opcode::DivSigned,
        Opcode::DivUnsigned,
        Opcode::RemSigned,
        Opcode::RemUnsigned,
    ] {
        for position in 0..4 {
            let mut f = Fixture::new(1, true);
            let left = f.field(f.address, 0);
            let right = f.field(f.address, 1);
            let one = f.graph.add_constant(1, f.word);
            let two = f.graph.add_constant(2, f.word);
            let trap_type = if matches!(op, Opcode::DivSigned | Opcode::RemSigned) {
                f.graph.oracle.intern(
                    TypeShape::Int {
                        width: IntegerWidth::W64,
                        signed: true,
                    },
                    Some(SizeAlign { size: 8, align: 8 }),
                )
            } else {
                f.word
            };
            let numerator = f.graph.add_constant(1, trap_type);
            let zero = f.graph.add_constant(0, trap_type);
            let old = f.store(left, one);
            if position == 0 {
                f.graph.add_node(
                    op,
                    f.control,
                    &[numerator, zero],
                    0,
                    ValueType::Rust(trap_type),
                );
            }
            let states = f.branch(f.conditions[0]);
            f.enter(states[0]);
            f.store(left, two);
            f.load_return(left);
            f.enter(states[1]);
            if position == 2 {
                f.straight_jump();
            } else if position == 3 {
                f.store(right, one);
            }
            if position != 0 {
                f.graph.add_node(
                    op,
                    f.control,
                    &[numerator, zero],
                    0,
                    ValueType::Rust(trap_type),
                );
            }
            f.store(left, two);
            f.load_return(left);
            assert_eq!(
                f.check(old, false).stores_removed,
                0,
                "{op:?} at {position}"
            );
            assert!(
                execute(&f.graph, &[Value::Scalar(0)], false)
                    .result
                    .is_err()
            );
        }
    }
}

#[test]
fn bypass_effect_consumers_after_the_overwrites_are_not_ignored() {
    for observation in [false, true] {
        let mut f = Fixture::new(1, true);
        let left = f.field(f.address, 0);
        let one = f.graph.add_constant(1, f.word);
        let old = f.store(left, one);
        let old_effect = f.effect;
        let states = f.branch(f.conditions[0]);
        for state in states {
            f.enter(state);
            f.store(left, one);
            let consumer = if observation {
                f.fixed(Opcode::RequireDefined, &[one], None).0
            } else {
                f.ret(one)
            };
            // Dominance permits this older token, but it is outside the
            // prefix that this bounded overwrite proof is allowed to erase.
            f.graph.set_input(consumer, 0, old_effect);
            if observation {
                f.ret(one);
            }
        }
        assert_eq!(f.check(old, false).stores_removed, 0);
    }
}

#[test]
fn a_store_with_a_different_incoming_effect_is_not_a_covered_edge() {
    let mut f = Fixture::new(1, true);
    let left = f.field(f.address, 0);
    let one = f.graph.add_constant(1, f.word);
    let start_effect = f.effect;
    let old = f.store(left, one);
    let states = f.branch(f.conditions[0]);
    for (index, state) in states.into_iter().enumerate() {
        f.enter(state);
        let store = f.store(left, one);
        if index == 1 {
            f.graph.set_input(store, 0, start_effect);
        }
        f.load_return(left);
    }
    assert_eq!(f.check(old, false).stores_removed, 0);
}

#[test]
fn extra_store_tuple_projections_reject_both_initial_and_intermediate_stores() {
    for intermediate in [false, true] {
        let mut f = Fixture::new(1, true);
        let left = f.field(f.address, 0);
        let right = f.field(f.address, 1);
        let one = f.graph.add_constant(1, f.word);
        let old = f.store(left, one);
        let source = if intermediate {
            f.store(right, one)
        } else {
            old
        };
        f.graph.add_node(
            Opcode::Projection,
            NodeId::NONE,
            &[source],
            1,
            ValueType::Effect,
        );
        let states = f.branch(f.conditions[0]);
        f.overwrite_arms(states, left, one);
        assert_eq!(f.check(old, false).stores_removed, 0);
    }
}

#[test]
fn a_bypass_effect_phi_after_both_overwrites_keeps_the_old_store() {
    let mut f = Fixture::new(1, true);
    let left = f.field(f.address, 0);
    let one = f.graph.add_constant(1, f.word);
    let old = f.store(left, one);
    let old_effect = f.effect;
    let states = f.branch(f.conditions[0]);
    let jumps = states.map(|state| {
        f.enter(state);
        f.store(left, one);
        f.jump()
    });
    let region = f
        .graph
        .add_node(Opcode::Region, NodeId::NONE, &jumps, 0, ValueType::Control);
    let phi = f.graph.add_node(
        Opcode::Phi,
        NodeId::NONE,
        &[region, old_effect, old_effect],
        0,
        ValueType::Effect,
    );
    f.enter((region, phi));
    f.load_return(left);
    assert_eq!(f.check(old, false).stores_removed, 0);
}

#[test]
fn joins_and_single_predecessor_phis_stop_the_control_walk() {
    for merge in [false, true] {
        let mut f = Fixture::new(1, true);
        let left = f.field(f.address, 0);
        let one = f.graph.add_constant(1, f.word);
        let old = f.store(left, one);
        let old_effect = f.effect;
        if merge {
            let states = f.branch(f.conditions[0]);
            let jumps = states.map(|state| {
                f.enter(state);
                f.jump()
            });
            f.control =
                f.graph
                    .add_node(Opcode::Region, NodeId::NONE, &jumps, 0, ValueType::Control);
            f.effect = old_effect;
        } else {
            f.straight_jump();
            f.effect = f.graph.add_node(
                Opcode::Phi,
                NodeId::NONE,
                &[f.control, old_effect],
                0,
                ValueType::Effect,
            );
        }
        let states = f.branch(f.conditions[0]);
        f.overwrite_arms(states, left, one);
        assert_eq!(f.check(old, false).stores_removed, 0);
    }
}

#[test]
fn a_loop_path_without_an_overwrite_keeps_the_old_store() {
    let mut f = Fixture::new(1, true);
    let left = f.field(f.address, 0);
    let one = f.graph.add_constant(1, f.word);
    let two = f.graph.add_constant(2, f.word);
    let old = f.store(left, one);
    let entry = f.jump();
    let region = f
        .graph
        .add_growable_node(Opcode::Region, NodeId::NONE, 2, 0, ValueType::Control);
    f.graph.set_input(region, 0, entry);
    f.control = region;
    let states = f.branch(f.conditions[0]);
    f.enter(states[0]);
    f.store(left, two);
    f.load_return(left);
    f.enter(states[1]);
    let back = f.jump();
    f.graph.set_input(region, 1, back);
    assert_eq!(f.check(old, false).stores_removed, 0);
    assert_eq!(
        execute(&f.graph, &[Value::Scalar(0)], false).result,
        Err(InterpretError::OutOfFuel)
    );
}

#[test]
fn raw_branch_overwrites_preserve_the_first_access_check() {
    for checked_before in [false, true] {
        let mut f = Fixture::new(1, false);
        let left = f.field(f.address, 0);
        let one = f.graph.add_constant(1, f.word);
        let two = f.graph.add_constant(2, f.word);
        let first = checked_before.then(|| f.store(left, one));
        let old = f.store(left, two);
        let states = f.branch(f.conditions[0]);
        f.overwrite_arms(states, left, one);
        let original = f.graph.clone();
        let stats = f.check(old, checked_before);
        assert_eq!(stats.stores_removed, usize::from(checked_before));
        if let Some(first) = first {
            assert!(!f.graph.is_dead(first));
        }
        for condition in [0, 1] {
            let input = [Value::Scalar(condition)];
            let before = execute(&original, &input, true);
            assert!(before.result.is_err());
            assert_eq!(execute(&f.graph, &input, true), before);
        }
    }
}

#[test]
fn whole_allocation_lifetimes_on_one_arm_stop_coverage_for_each_field() {
    for field in [0, 1] {
        for events in [
            &[Opcode::SlotDead][..],
            &[Opcode::SlotLive][..],
            &[Opcode::SlotDead, Opcode::SlotLive][..],
        ] {
            let mut f = Fixture::new(1, true);
            let root = f.local();
            let address = f.field(root, field);
            let one = f.graph.add_constant(1, f.word);
            let two = f.graph.add_constant(2, f.word);
            let old = f.store(address, one);
            let states = f.branch(f.conditions[0]);
            f.enter(states[0]);
            f.store(address, two);
            f.load_return(address);
            f.enter(states[1]);
            for &event in events {
                f.fixed(event, &[root], None);
            }
            f.store(address, two);
            f.load_return(address);
            assert_eq!(f.check(old, false).stores_removed, 0);
        }
    }
}
