#[path = "protected_fixture.rs"]
mod fixture;
use fixture::*;
use kfrc_body::{FunctionAbi, Mutability, PassMode, SizeAlign, TypeShape};
use kfrc_son::{ArgPermission, Opcode, ValueType};

#[test]
fn protected_snapshots_keep_unrelated_writes_calls_and_all_callers() {
    for kind in [
        Kind::Word,
        Kind::Pointer,
        Kind::Reference,
        Kind::Str,
        Kind::Slice,
    ] {
        for frozen in [false, true] {
            let f = Fixture::new(kind, frozen, false);
            let mut graphs = f.graphs();
            graphs.insert("other".into(), caller(&f.f, "other"));
            let before = graphs.clone();
            let stats = promote(&mut graphs);
            assert_eq!(stats.arguments_promoted, usize::from(enabled()), "{kind:?}");
            assert_eq!(stats.protected_arguments, usize::from(enabled()));
            assert_eq!(stats.calls_changed, if enabled() { 2 } else { 0 });
            assert_eq!(stats.pair_parts, usize::from(enabled() && kind.wide()));
            if enabled() {
                let source = &graphs["protected_reader"];
                assert_eq!(
                    source.abi.args[0],
                    if kind.wide() {
                        PassMode::Pair
                    } else {
                        PassMode::Direct
                    }
                );
                assert_eq!(source.arg_permissions[0], ArgPermission::None);
                assert!(
                    !source
                        .ids()
                        .any(|node| source.op(node) == Opcode::LoadValue)
                );
                assert_eq!(
                    source
                        .ids()
                        .filter(|&n| source.op(n) == Opcode::StoreValue)
                        .count(),
                    1
                );
                assert_eq!(
                    source
                        .ids()
                        .filter(|&n| source.op(n) == Opcode::Call)
                        .count(),
                    3
                );
            }
            for failure in [Failure::None, Failure::Callback, Failure::Diverges] {
                for poison in [None, Some(0)] {
                    assert_eq!(
                        execute(&before, kind, poison, failure, false, false),
                        execute(&graphs, kind, poison, failure, false, false),
                        "{kind:?} {failure:?} {poison:?}"
                    );
                }
            }
            if kind.wide() {
                assert_eq!(
                    execute(&before, kind, Some(1), Failure::None, false, false),
                    execute(&graphs, kind, Some(1), Failure::None, false, false)
                );
            }
            assert_eq!(
                execute(&before, kind, Some(0), Failure::None, false, true),
                execute(&graphs, kind, Some(0), Failure::None, false, true)
            );
        }
    }
}

#[test]
fn partial_wide_observation_does_not_force_the_unobserved_poison_component() {
    for kind in [Kind::Str, Kind::Slice] {
        let f = Fixture::new(kind, false, true);
        let mut graphs = f.graphs();
        let before = graphs.clone();
        assert_eq!(promote(&mut graphs).arguments_promoted, 0);
        let run = execute(&graphs, kind, Some(0), Failure::None, false, false);
        assert_eq!(
            run,
            execute(&before, kind, Some(0), Failure::None, false, false)
        );
        assert!(run.outcome.is_ok(), "{run:?}");
    }
}

#[test]
fn producer_wide_reconstruction_is_canonicalized_before_argument_promotion() {
    for kind in [Kind::Str, Kind::Slice] {
        for keep_data in [false, true] {
            let mut f = Fixture::new(kind, false, false);
            let calls: Vec<_> =
                f.f.graph
                    .ids()
                    .filter(|&node| {
                        f.f.graph.op(node) == Opcode::Call
                            && f.f.graph.callees[f.f.graph.payload(node) as usize].symbol
                                == "observe_snapshot"
                    })
                    .collect();
            for call in calls {
                let value = f.f.graph.input(call, 1);
                let rebuilt = reconstruct_wide(&mut f.f, value, keep_data);
                f.f.graph.set_input(call, 1, rebuilt);
            }
            f.f.verify();
            // The crate's source preparation performs this S pipeline before
            // the ABI transaction. Reconstructed complete values simplify to
            // the entry load; a partial reconstruction must keep its poison.
            let raw = f.graphs();
            let mut unprepared = raw.clone();
            assert_eq!(promote(&mut unprepared).arguments_promoted, 0);
            kfrc_son::optimize_specialized_graph(&mut f.f.graph).unwrap();
            let mut prepared = f.graphs();
            let before = prepared.clone();
            let stats = promote(&mut prepared);
            assert_eq!(
                stats.arguments_promoted,
                usize::from(enabled() && keep_data),
                "{kind:?} complete={keep_data}"
            );
            for failure in [Failure::None, Failure::Callback, Failure::Diverges] {
                for poison in [None, Some(0), Some(1)] {
                    let expected = execute(&raw, kind, poison, failure, false, false);
                    assert_eq!(
                        expected,
                        execute(&before, kind, poison, failure, false, false)
                    );
                    assert_eq!(
                        expected,
                        execute(&prepared, kind, poison, failure, false, false)
                    );
                }
            }
        }
    }
}

#[test]
fn missing_or_inexact_protectors_and_raw_roots_keep_the_original_alias_reads() {
    for bytes in [None, Some(0), Some(8), Some(17)] {
        let mut f = Fixture::new(Kind::Word, false, false);
        f.f.graph.arg_permissions[0] = ArgPermission::Unique {
            dereferenceable: bytes,
        };
        // Such metadata is not producer-authenticated: the planner must reject
        // it independently of whether the optional verifier also rejects it.
        let mut graphs = f.graphs();
        let before = graphs.clone();
        assert_eq!(promote(&mut graphs).arguments_promoted, 0);
        assert_eq!(
            execute(&before, Kind::Word, None, Failure::None, true, false),
            execute(&graphs, Kind::Word, None, Failure::None, true, false)
        );
    }
    for raw in [false, true] {
        let mut f = Fixture::new(Kind::Word, false, false);
        f.f.graph.arg_permissions[0] = ArgPermission::None;
        if raw {
            let ty = f.f.graph.oracle.intern(
                TypeShape::RawPointer {
                    pointee: f.container,
                    mutability: Mutability::Mut,
                },
                Some(SizeAlign { size: 8, align: 8 }),
            );
            let ValueType::Tuple(mut params) = f.f.graph.value_type(f.f.graph.start).clone() else {
                unreachable!()
            };
            params[2] = ValueType::Rust(ty);
            f.f.graph
                .set_value_type(f.f.graph.start, ValueType::Tuple(params));
            f.f.graph.set_value_type(f.f.args[0], ValueType::Rust(ty));
        }
        let mut graphs = f.graphs();
        let before = graphs.clone();
        assert_eq!(promote(&mut graphs).arguments_promoted, 0);
        let run = execute(&graphs, Kind::Word, None, Failure::None, true, false);
        assert_eq!(
            run,
            execute(&before, Kind::Word, None, Failure::None, true, false)
        );
        assert!(run.outcome.is_ok());
    }
}

#[test]
fn root_escape_and_barriers_block_replacement_of_later_reads() {
    for case in [
        "call",
        "store",
        "opaque",
        "tls",
        "pointer_bits",
        "slot_dead",
    ] {
        let mut f = Fixture::new(Kind::Word, false, false);
        match case {
            "call" => {
                let mut inputs = f.f.graph.inputs(f.touch).to_vec();
                inputs[1] = f.f.args[0];
                f.f.graph.set_inputs(f.touch, &inputs);
            }
            "store" => {
                let second_load = f.f.graph.input(f.second, 0);
                f.f.control = f.f.graph.control(second_load);
                f.f.effect = f.f.graph.input(second_load, 0);
                let value = f.f.constant(51);
                store(&mut f.f, f.address, value);
                f.f.graph.set_control(second_load, f.f.control);
                f.f.graph.set_input(second_load, 0, f.f.effect);
            }
            "opaque" => {
                f.f.graph.add_node(
                    Opcode::Opaque,
                    f.f.control,
                    &[f.f.effect],
                    0,
                    ValueType::Tuple(vec![ValueType::Control, ValueType::Effect]),
                );
            }
            "tls" => {
                let payload = f.f.graph.symbols.len() as u32;
                f.f.graph.symbols.push(("tls".into(), 0));
                f.f.node(Opcode::ThreadLocalAddress, &[], payload, f.root_ref);
            }
            "pointer_bits" => {
                f.f.node(Opcode::IntCast, &[f.f.args[0]], 0, f.f.word);
            }
            "slot_dead" => {
                f.f.graph.add_node(
                    Opcode::SlotDead,
                    f.f.control,
                    &[f.f.effect, f.address],
                    0,
                    ValueType::Tuple(vec![ValueType::Control, ValueType::Effect]),
                );
            }
            _ => unreachable!(),
        }
        let source = f.f.graph.clone();
        let mut graphs = std::collections::BTreeMap::from([
            (source.symbol.clone(), source),
            ("caller".into(), caller(&f.f, "caller")),
        ]);
        let stats = kfrc_son::promote_arguments(
            &mut graphs,
            &std::collections::BTreeSet::from(["protected_reader".into()]),
        )
        .unwrap();
        assert_eq!(stats.functions_changed, 0, "{case}");
    }
}

#[test]
fn branch_before_snapshot_and_stale_call_signature_abort_the_plan() {
    let mut f = Fixture::new(Kind::Word, false, false);
    let first_load = f.f.graph.input(f.first, 0);
    let condition_ty =
        f.f.graph
            .oracle
            .intern(TypeShape::Bool, Some(SizeAlign { size: 1, align: 1 }));
    let condition = f.f.graph.add_constant(1, condition_ty);
    let branch = f.f.graph.add_node(
        Opcode::If,
        f.f.graph.control(first_load),
        &[condition],
        0,
        ValueType::Tuple(vec![ValueType::Control; 2]),
    );
    let true_arm = crate::fixture::projection(&mut f.f.graph, branch, 0, ValueType::Control);
    f.f.graph.set_control(first_load, true_arm);
    let mut graphs = f.graphs();
    assert_eq!(promote(&mut graphs).arguments_promoted, 0);

    let f = Fixture::new(Kind::Str, false, false);
    let mut graphs = f.graphs();
    let owner = graphs.get_mut("caller").unwrap();
    let call = owner.ids().find(|&n| owner.op(n) == Opcode::Call).unwrap();
    let target = owner.payload(call) as usize;
    owner.callees[target].abi = FunctionAbi {
        args: vec![PassMode::Direct],
        ret: PassMode::Direct,
        c_variadic: false,
    };
    let before: Vec<_> = graphs
        .iter()
        .map(|(s, g)| (s.clone(), g.dump(), g.abi.clone()))
        .collect();
    let stats = kfrc_son::promote_arguments(
        &mut graphs,
        &std::collections::BTreeSet::from(["protected_reader".into()]),
    )
    .unwrap();
    assert_eq!(stats.functions_changed, 0);
    assert_eq!(
        before,
        graphs
            .iter()
            .map(|(s, g)| (s.clone(), g.dump(), g.abi.clone()))
            .collect::<Vec<_>>()
    );
}

#[test]
fn entry_require_defined_and_access_ranges_preserve_snapshot_proofs() {
    for case in [
        "observe",
        "misaligned",
        "outside",
        "conditional_second_field",
    ] {
        let mut f = Fixture::new(Kind::Word, false, false);
        match case {
            "observe" => {
                let first_call =
                    f.f.graph
                        .ids()
                        .find(|&n| f.f.graph.op(n) == Opcode::Call)
                        .unwrap();
                f.f.control = f.f.graph.control(first_call);
                f.f.effect = f.f.graph.input(first_call, 0);
                f.f.observe(f.first);
                f.f.graph.set_control(first_call, f.f.control);
                f.f.graph.set_input(first_call, 0, f.f.effect);
            }
            "misaligned" | "outside" => {
                let mut layout = f.f.graph.oracle.aggregates[&f.container].clone();
                layout.fields[1].offset = if case == "misaligned" { 1 } else { 16 };
                f.f.graph.oracle.record_aggregate(f.container, layout);
            }
            "conditional_second_field" => {
                // This additional field is read only after an external effect,
                // so passing it as a defined argument would be premature.
                let address_ty = f.f.graph.value_type(f.address).clone();
                let other = f.f.graph.add_node(
                    Opcode::FieldAddr,
                    kfrc_son::NodeId::NONE,
                    &[f.f.graph.input(f.address, 0)],
                    0,
                    address_ty,
                );
                let second_load = f.f.graph.input(f.second, 0);
                f.f.graph.set_input(second_load, 1, other);
            }
            _ => unreachable!(),
        }
        let mut graphs = f.graphs();
        let before = graphs.clone();
        let stats = kfrc_son::promote_arguments(
            &mut graphs,
            &std::collections::BTreeSet::from(["protected_reader".into()]),
        )
        .unwrap();
        assert_eq!(
            stats.arguments_promoted,
            usize::from(case == "observe" && enabled()),
            "{case}"
        );
        if case == "observe" {
            for poison in [None, Some(0)] {
                assert_eq!(
                    execute(&before, Kind::Word, poison, Failure::None, false, false),
                    execute(&graphs, Kind::Word, poison, Failure::None, false, false)
                );
            }
        }
    }
}

#[test]
fn replacing_the_pointer_argument_exposes_caller_storage_to_scalar_promotion() {
    let source = Fixture::new(Kind::Word, false, false);
    let mut caller_graph = caller(&source.f, "caller");
    let ids: Vec<_> = caller_graph.ids().collect();
    for node in ids {
        if node != caller_graph.start
            && !(caller_graph.op(node) == Opcode::Projection
                && caller_graph.input(node, 0) == caller_graph.start)
        {
            caller_graph.remove_node(node);
        }
    }
    let mut f = crate::fixture::Function {
        word: source.f.word,
        control: caller_graph.find_projection(caller_graph.start, 0).unwrap(),
        effect: caller_graph.find_projection(caller_graph.start, 1).unwrap(),
        args: (2..4)
            .map(|i| caller_graph.find_projection(caller_graph.start, i).unwrap())
            .collect(),
        graph: caller_graph,
    };
    // Keep the original out parameter, but construct Parser storage locally.
    let place = f.graph.oracle.intern(
        TypeShape::PlaceAddress {
            pointee: source.container,
        },
        Some(SizeAlign { size: 8, align: 8 }),
    );
    let slot = f.node(Opcode::LocalSlot, &[], 47, place);
    let a = f.constant(31);
    let b = f.constant(17);
    let value = f.node(Opcode::Aggregate, &[a, b], 0, source.container);
    store(&mut f, slot, value);
    let root = f.node(Opcode::PointerCast, &[slot], 0, source.root_ref);
    let out = f.args[1];
    let result = f.call(&source.f.graph, &[root, out]);
    f.ret(result);
    f.verify();
    let mut graphs = source.graphs();
    graphs.insert("caller".into(), f.graph);
    let before = graphs.clone();
    let stats = promote(&mut graphs);
    assert_eq!(stats.arguments_promoted, usize::from(enabled()));
    let caller = graphs.get_mut("caller").unwrap();
    let optimized = kfrc_son::optimize_specialized_graph(caller).unwrap();
    assert_eq!(optimized.slots_promoted > 0, enabled());
    // RequireDefined retains the original pointer observation and may keep a
    // LocalSlot identity alive. The proof is that its memory traffic vanishes.
    let memory = kfrc_son::analysis::memory::MemoryFacts::new(caller);
    assert_eq!(
        caller.ids().any(|node| {
            matches!(caller.op(node), Opcode::LoadValue | Opcode::StoreValue)
                && memory.root(caller.input(node, 1)).is_some_and(|root| {
                    caller.op(root) == Opcode::LocalSlot && caller.payload(root) == 47
                })
        }),
        !enabled()
    );
    for failure in [Failure::None, Failure::Callback, Failure::Diverges] {
        assert_eq!(
            execute(&before, Kind::Word, None, failure, false, false),
            execute(&graphs, Kind::Word, None, failure, false, false)
        );
    }
}
