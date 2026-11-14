//! Cyclic-call profitability must preserve data identity and complete behavior.
#[path = "semantic_inline_preparation/fixture.rs"]
mod fixture;
use fixture::{Function, equivalent, projection};
use kfrc_body::fx_hash::HashMap;
use kfrc_body::{DataObject, DataRelocation, SizeAlign, TypeShape};
use kfrc_son::{
    ComparePredicate, FunctionGraph, GraphProvider, InlineBudget, InlineSkip, NodeId, Opcode,
    Value, ValueType, inline_calls,
};

fn constant_user(name: &str, bytes: usize) -> Function {
    let mut f = Function::new(name, 1);
    let address = f.address(&format!("{name}_data"), false);
    f.graph.data_objects[0].unnamed_address = true;
    f.graph.data_objects[0].bytes.resize(bytes, 0x5a);
    // Keep the symbol alive without requiring a value load or granting any
    // assumption about the identity or contents of unrelated memory.
    f.observe(address);
    let seven = f.constant(7);
    let result = f.node(Opcode::Add, &[f.args[0], seven], 0, f.word);
    f.ret(result);
    f.verify();
    f
}

fn caller(callees: &[&FunctionGraph], cyclic: bool) -> Function {
    let mut f = Function::new("constant_loop", 2);
    if !cyclic {
        let mut result = f.args[0];
        for callee in callees {
            result = f.call(callee, &[result]);
        }
        f.ret(result);
        f.verify();
        return f;
    }
    let boolean = f
        .graph
        .oracle
        .intern(TypeShape::Bool, Some(SizeAlign { size: 1, align: 1 }));
    let zero = f.constant(0);
    let one = f.constant(1);
    let entry = f
        .graph
        .add_node(Opcode::Jump, f.control, &[], 0, ValueType::Control);
    let header = f.graph.add_node(
        Opcode::Region,
        NodeId::NONE,
        &[entry],
        0,
        ValueType::Control,
    );
    let iv = f.graph.add_node(
        Opcode::Phi,
        NodeId::NONE,
        &[header, zero],
        0,
        ValueType::Rust(f.word),
    );
    let result = f.graph.add_node(
        Opcode::Phi,
        NodeId::NONE,
        &[header, f.args[0]],
        0,
        ValueType::Rust(f.word),
    );
    let effect = f.graph.add_node(
        Opcode::Phi,
        NodeId::NONE,
        &[header, f.effect],
        0,
        ValueType::Effect,
    );
    let condition = f.node(
        Opcode::Compare,
        &[iv, f.args[1]],
        ComparePredicate::UnsignedLess as u32,
        boolean,
    );
    let branch = f.graph.add_node(
        Opcode::If,
        header,
        &[condition],
        0,
        ValueType::Tuple(vec![ValueType::Control; 2]),
    );
    let body = projection(&mut f.graph, branch, 0, ValueType::Control);
    let exit = projection(&mut f.graph, branch, 1, ValueType::Control);
    f.control = body;
    f.effect = effect;
    let mut next_result = result;
    for callee in callees {
        next_result = f.call(callee, &[next_result]);
    }
    let next_iv = f.node(Opcode::Add, &[iv, one], 0, f.word);
    let latch = f
        .graph
        .add_node(Opcode::Jump, f.control, &[], 0, ValueType::Control);
    f.graph.set_inputs(header, &[entry, latch]);
    f.graph.set_inputs(iv, &[header, zero, next_iv]);
    f.graph
        .set_inputs(result, &[header, f.args[0], next_result]);
    f.graph
        .set_inputs(effect, &[header, f.graph.input(effect, 1), f.effect]);
    f.control = exit;
    f.effect = effect;
    f.ret(result);
    f.verify();
    f
}

fn equivalent_inputs(
    before: &FunctionGraph,
    after: &FunctionGraph,
    graphs: &HashMap<String, FunctionGraph>,
) {
    for seed in [0, 1, u64::MAX, u64::MAX - 17, 0x8123_4567_89ab_cdef] {
        for count in [0, 1, 2, 7, 31] {
            equivalent(
                before,
                after,
                graphs,
                &[Value::Scalar(seed), Value::Scalar(count)],
            );
        }
    }
}

#[test]
fn cyclic_profitability_never_moves_a_failing_observation_before_the_guard() {
    let mut source = constant_user("poison_leaf", 8);
    let ret = source
        .graph
        .ids()
        .find(|&node| source.graph.op(node) == Opcode::Return)
        .unwrap();
    let poison = source.node(Opcode::Undefined, &[], 0, source.word);
    source.observe(poison);
    source.graph.set_control(ret, source.control);
    source
        .graph
        .set_inputs(ret, &[source.effect, source.args[0]]);
    source.verify();
    let mut f = caller(&[&source.graph], true);
    let before = f.graph.clone();
    let graphs = HashMap::from_iter([(source.graph.symbol.clone(), source.graph)]);
    let stats = inline_calls(&mut f.graph, &graphs, InlineBudget::default()).unwrap();
    assert_eq!(stats.calls_inlined, 1);
    f.verify();
    equivalent_inputs(&before, &f.graph, &graphs);
}

#[test]
fn cyclic_sites_and_nested_calls_import_only_within_one_storage_budget() {
    let source = constant_user("leaf", 8);
    let mut wrapper = Function::new("wrapper", 1);
    let result = wrapper.call(&source.graph, &[wrapper.args[0]]);
    wrapper.ret(result);
    wrapper.verify();
    for target in [&source.graph, &wrapper.graph] {
        for cyclic in [false, true] {
            let mut f = caller(&[target], cyclic);
            let before = f.graph.clone();
            let graphs = HashMap::from_iter([
                (source.graph.symbol.clone(), source.graph.clone()),
                (wrapper.graph.symbol.clone(), wrapper.graph.clone()),
            ]);
            let stats = inline_calls(&mut f.graph, &graphs, InlineBudget::default()).unwrap();
            f.verify();
            assert_eq!(stats.constant_bytes_imported, if cyclic { 15 } else { 0 });
            assert!(stats.loop_constant_work <= InlineBudget::default().max_nodes * 8);
            let calls = f
                .graph
                .ids()
                .filter(|&n| f.graph.op(n) == Opcode::Call)
                .count();
            assert_eq!(calls, usize::from(!cyclic));
            equivalent_inputs(&before, &f.graph, &graphs);
        }
    }
}

#[test]
fn repeated_constants_are_reused_and_distinct_imports_share_the_limit() {
    let a = constant_user("a", 8);
    let b = constant_user("b", 8);
    let graphs = HashMap::from_iter([
        (a.graph.symbol.clone(), a.graph.clone()),
        (b.graph.symbol.clone(), b.graph.clone()),
    ]);
    for targets in [[&a.graph, &a.graph], [&a.graph, &b.graph]] {
        let mut f = caller(&targets, true);
        let before = f.graph.clone();
        let stats = inline_calls(
            &mut f.graph,
            &graphs,
            InlineBudget {
                max_loop_constant_bytes: 15,
                ..InlineBudget::default()
            },
        )
        .unwrap();
        f.verify();
        assert_eq!(stats.constant_bytes_imported, 15);
        assert_eq!(
            stats.calls_inlined,
            if targets[0].symbol == targets[1].symbol {
                2
            } else {
                1
            }
        );
        equivalent_inputs(&before, &f.graph, &graphs);
    }
    for bytes in [0, 1, 14] {
        let mut f = caller(&[&a.graph], true);
        let before = f.graph.clone();
        let stats = inline_calls(
            &mut f.graph,
            &graphs,
            InlineBudget {
                max_loop_constant_bytes: bytes,
                ..InlineBudget::default()
            },
        )
        .unwrap();
        assert_eq!(stats.calls_inlined, 0);
        assert_eq!(stats.constant_bytes_imported, 0);
        assert_eq!(f.graph.dump(), before.dump());
    }
}

#[test]
fn object_identity_tls_conflicts_and_relocation_targets_are_still_checked() {
    for kind in 0..6 {
        let mut source = constant_user("unsafe_import", 8);
        match kind {
            0 => source.graph.data_objects[0].writable = true,
            1 => {
                source.graph.data_objects.push(DataObject {
                    symbol: "tls_child".into(),
                    bytes: vec![0; 8],
                    align: 8,
                    thread_local: true,
                    unnamed_address: true,
                    ..Default::default()
                });
                source.graph.data_objects[0]
                    .relocations
                    .push(DataRelocation {
                        offset: 0,
                        target: "tls_child".into(),
                        addend: 0,
                    });
            }
            2 => source.graph.data_objects[0].unnamed_address = false,
            3 | 4 => {
                source.address("identity_child", kind == 3);
                source.graph.data_objects[0]
                    .relocations
                    .push(DataRelocation {
                        offset: 0,
                        target: "identity_child".into(),
                        addend: 0,
                    });
            }
            5 => {}
            _ => unreachable!(),
        }
        if kind == 1 {
            // Ordinary relocations to TLS are not legal source graphs. This
            // boundary must reject the fixture before any inlining policy.
            assert!(matches!(
                kfrc_son::verify_graph(&mut source.graph),
                Err(kfrc_son::VerifyError::BadPayload { detail, .. })
                    if detail.contains("cannot target TLS")
            ));
            continue;
        }
        source.verify();
        let mut f = caller(&[&source.graph], true);
        if kind == 5 {
            let mut conflicting = source.graph.data_objects[0].clone();
            conflicting.bytes[0] ^= 1;
            f.graph.data_objects.push(conflicting);
        }
        let before = f.graph.dump();
        let objects_before = f.graph.data_objects.clone();
        let graphs = HashMap::from_iter([(source.graph.symbol.clone(), source.graph)]);
        let stats = inline_calls(&mut f.graph, &graphs, InlineBudget::default()).unwrap();
        assert_eq!(stats.calls_inlined, 0);
        assert_eq!(stats.constant_bytes_imported, 0);
        assert!(
            stats
                .skipped
                .iter()
                .any(|(_, skip)| *skip == InlineSkip::DataConflict)
        );
        assert_eq!(f.graph.dump(), before);
        assert_eq!(f.graph.data_objects, objects_before);
    }
}

#[test]
fn cycle_analysis_exhaustion_preserves_the_original_call() {
    let source = constant_user("leaf", 8);
    let mut f = caller(&[&source.graph], true);
    for value in 0..1024 {
        f.constant(value);
    }
    f.verify();
    let before = f.graph.clone();
    let graphs = HashMap::from_iter([(source.graph.symbol.clone(), source.graph)]);
    let stats = inline_calls(
        &mut f.graph,
        &graphs,
        InlineBudget {
            max_nodes: 64,
            ..InlineBudget::default()
        },
    )
    .unwrap();
    assert_eq!((stats.calls_inlined, stats.constant_bytes_imported), (0, 0));
    assert_eq!(stats.loop_constant_work, 64 * 8);
    assert_eq!(f.graph.dump(), before.dump());
    equivalent_inputs(&before, &f.graph, &graphs);
}

struct Visibility<'a>(&'a HashMap<String, FunctionGraph>);
impl GraphProvider for Visibility<'_> {
    fn graph(&self, symbol: &str) -> Option<&FunctionGraph> {
        self.0.get(symbol)
    }
    fn can_reference(&self, symbol: &str) -> bool {
        symbol != "private_target"
    }
}

#[test]
fn failed_plans_are_transactional_and_keep_original_visibility_checks() {
    let mut bad = constant_user("bad", 8);
    bad.graph.data_objects[0].relocations.push(DataRelocation {
        offset: 0,
        target: "private_target".into(),
        addend: 0,
    });
    let good = constant_user("good", 8);
    let graphs = HashMap::from_iter([
        (bad.graph.symbol.clone(), bad.graph.clone()),
        (good.graph.symbol.clone(), good.graph.clone()),
    ]);
    let mut f = caller(&[&bad.graph, &good.graph], true);
    let stats = inline_calls(
        &mut f.graph,
        &Visibility(&graphs),
        InlineBudget {
            max_loop_constant_bytes: 15,
            ..InlineBudget::default()
        },
    )
    .unwrap();
    f.verify();
    assert_eq!(
        (stats.calls_inlined, stats.constant_bytes_imported),
        (1, 15)
    );
    assert_eq!(f.graph.data_objects.len(), 1);
    assert_eq!(f.graph.data_objects[0].symbol, "good_data");
    assert!(
        stats
            .skipped
            .iter()
            .any(|(_, skip)| *skip == InlineSkip::PrivateReference)
    );
}
