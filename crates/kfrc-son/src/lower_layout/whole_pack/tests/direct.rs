use super::*;

// Once an enum wrapper is simplified, the same complete pack consumes the
// guarded load results directly, without an inactive-value Phi per byte.
fn direct(mut f: Fixture) -> Fixture {
    for &phi in &f.phis {
        let value = f.graph.input(phi, f.slot);
        f.graph.replace(phi, value);
    }
    // The direct pack is computed on the load path. Only its whole result
    // crosses the join; a branch-local byte cannot feed a post-join cast.
    let poison = f.graph.add_node(
        Opcode::Undefined,
        NodeId::NONE,
        &[],
        0,
        ValueType::Rust(f.word),
    );
    let mut inputs = [f.region, poison, poison];
    inputs[f.slot] = f.pack;
    let value = f.graph.add_node(
        Opcode::Phi,
        NodeId::NONE,
        &inputs,
        0,
        ValueType::Rust(f.word),
    );
    let returned = f.graph.input(f.ret, 1);
    f.graph.set_input(returned, 0, value);
    f.graph.resolve_edges();
    f
}

#[test]
fn direct_guarded_loads_keep_width_poison_snapshot_and_inactive_path() {
    for bytes in [2, 4, 8] {
        for balanced in [false, true] {
            for missing in std::iter::once(None).chain((0..bytes).map(Some)) {
                let mut f = direct(Fixture::new(bytes, 1, balanced, missing, 1));
                f.overwrite_after_join();
                verify_graph(&mut f.graph).unwrap();
                let before = f.graph.clone();
                let stats = run(&mut f.graph, 128);
                assert_eq!(stats.groups, 1, "{}", before.dump());
                assert_eq!(stats.byte_loads_removed, bytes - 1);
                assert_eq!(stats.payload_phis_removed, 0);
                verify_graph(&mut f.graph).unwrap();
                let loads: Vec<_> = f
                    .graph
                    .ids()
                    .filter(|&n| f.graph.op(n) == Opcode::Load)
                    .collect();
                assert_eq!(loads.len(), 1);
                let control = f.graph.control(loads[0]);
                assert_eq!(f.graph.op(control), Opcode::Projection);
                assert_eq!(f.graph.op(f.graph.input(control, 0)), Opcode::If);
                if missing.is_none() {
                    assert_eq!(evaluate(&f.graph, true), Ok(Outcome::Return(f.expected())));
                } else {
                    assert!(matches!(
                        evaluate(&f.graph, true),
                        Err(InterpretError::PoisonObserved(_))
                    ));
                }
                assert_eq!(
                    evaluate(&f.graph, false),
                    Ok(Outcome::Return(fixture::SENTINEL))
                );
                same_execution(&before, &f.graph, true);
                same_execution(&before, &f.graph, false);
            }
        }
    }
}

#[test]
fn direct_pack_never_speculates_an_inactive_poisoned_pointer() {
    let mut f = direct(Fixture::new(8, 2, true, None, 0));
    let invalid = f.graph.add_node(
        Opcode::Undefined,
        NodeId::NONE,
        &[],
        0,
        ValueType::Rust(f.pointer),
    );
    for &address in &f.addresses {
        f.graph.set_input(address, 0, invalid);
    }
    verify_graph(&mut f.graph).unwrap();
    let before = f.graph.clone();
    assert_eq!(run(&mut f.graph, 128).groups, 1);
    verify_graph(&mut f.graph).unwrap();
    same_execution(&before, &f.graph, false);
    same_execution(&before, &f.graph, true);
}

#[test]
fn direct_pack_rejects_interleaved_writes_and_individual_byte_observers() {
    for interleaved in [false, true] {
        let mut f = direct(Fixture::new(8, 1, false, None, 0));
        if interleaved {
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
            let control = fixture::project(&mut f.graph, store, 0, ValueType::Control);
            let effect = fixture::project(&mut f.graph, store, 1, ValueType::Effect);
            f.graph.set_control(next, control);
            f.graph.set_input(next, 0, effect);
        } else {
            let value = f.graph.find_projection(f.loads[3], 2).unwrap();
            f.graph.add_node(
                Opcode::IntCast,
                NodeId::NONE,
                &[value],
                0,
                ValueType::Rust(f.word),
            );
        }
        verify_graph(&mut f.graph).unwrap();
        let before = f.graph.dump();
        assert_eq!(run(&mut f.graph, 128), Stats::default());
        assert_eq!(f.graph.dump(), before);
    }
}
