//! The control producer changes compile-time work order, never runtime effects
//! or the emitted caller's independent size/depth policy.
use super::*;
use kfrc_body::{Mutability, SizeAlign, TyId, TypeShape};
use kfrc_son::{ComparePredicate, ValueType};

fn scalar_gate() -> Function {
    let mut f = Function::new("scalar_gate", 1);
    let boolean = f
        .graph
        .oracle
        .intern(TypeShape::Bool, Some(SizeAlign { size: 1, align: 1 }));
    let limit = f.constant(128);
    let condition = f.node(
        Opcode::Compare,
        &[f.args[0], limit],
        ComparePredicate::UnsignedLess as u32,
        boolean,
    );
    let branch = f.graph.add_node(
        Opcode::If,
        f.control,
        &[condition],
        0,
        ValueType::Tuple(vec![ValueType::Control; 2]),
    );
    f.control = projection(&mut f.graph, branch, 0, ValueType::Control);
    let one = f.constant(1);
    f.ret(one);
    f.control = projection(&mut f.graph, branch, 1, ValueType::Control);
    let two = f.constant(2);
    f.ret(two);
    f.verify();
    f
}

fn pointer(f: &mut Function) -> TyId {
    f.graph.oracle.intern(
        TypeShape::RawPointer {
            mutability: Mutability::Not,
            pointee: f.word,
        },
        Some(SizeAlign { size: 8, align: 8 }),
    )
}

fn address_helper() -> Function {
    let mut f = Function::new("address_helper", 1);
    let pointer = pointer(&mut f);
    f.graph.symbols.push(("opaque_address".into(), 0));
    let address = f.node(Opcode::SymbolAddress, &[], 0, pointer);
    f.ret(address);
    f.verify();
    f
}

fn branching(gate: &Function, address: &Function) -> Function {
    let mut f = Function::new("branching", 2);
    let boolean = f
        .graph
        .oracle
        .intern(TypeShape::Bool, Some(SizeAlign { size: 1, align: 1 }));
    let tag = f.args[1];
    let selected = f.call(&gate.graph, &[tag]);
    let one = f.constant(1);
    let different = f.node(
        Opcode::Compare,
        &[selected, one],
        ComparePredicate::NotEqual as u32,
        boolean,
    );
    let condition = f.node(Opcode::BitNot, &[different], 0, boolean);
    let branch = f.graph.add_node(
        Opcode::If,
        f.control,
        &[condition],
        0,
        ValueType::Tuple(vec![ValueType::Control; 2]),
    );
    let entry_effect = f.effect;
    f.control = projection(&mut f.graph, branch, 0, ValueType::Control);
    f.ret(f.args[0]);
    f.control = projection(&mut f.graph, branch, 1, ValueType::Control);
    f.effect = entry_effect;
    let pointer = pointer(&mut f);
    let mut value = f.args[0];
    for _ in 0..9 {
        let result = f.call_named_typed(&address.graph.symbol, &address.graph.abi, &[tag], pointer);
        // These effects are observable on the dynamic non-ASCII path; the
        // priority analysis must not execute or hoist any of them.
        value = f.call_named("touch", &address.graph.abi, &[result]);
    }
    f.ret(value);
    f.verify();
    assert!(f.graph.live_count() > 64 && f.graph.len() <= 512);
    f
}

#[test]
fn preparation_prunes_cold_address_helpers_before_copying_their_sources() {
    let gate = scalar_gate();
    let address = address_helper();
    assert!(address.graph.live_count() < gate.graph.live_count());
    let source = branching(&gate, &address);
    let graphs = HashMap::from_iter([
        (source.graph.symbol.clone(), source.graph.clone()),
        (gate.graph.symbol.clone(), gate.graph),
        (address.graph.symbol.clone(), address.graph),
    ]);
    let mut f = caller(&source.graph, Some(32), 1);
    let before = f.graph.clone();
    let stats = inline_calls(&mut f.graph, &graphs, InlineBudget::default()).unwrap();
    assert!(stats.preparation.priority_node_visits > 0);
    assert!(stats.preparation.decisions.iter().any(|decision| {
        decision.symbol == "scalar_gate"
            && decision.action == "splice"
            && decision.rejection.is_none()
    }));
    assert!(
        stats
            .preparation
            .decisions
            .iter()
            .all(|decision| { decision.symbol != "address_helper" && decision.symbol != "touch" })
    );
    assert!(calls(&f.graph).is_empty());
    f.verify();
    equivalent(&before, &f.graph, &graphs, &[Value::Scalar(7)]);
    assert!(execute(&f.graph, &graphs, &[Value::Scalar(7)]).1.is_empty());
}

#[test]
fn emission_keeps_size_order_for_the_same_control_and_address_shape() {
    let gate = scalar_gate();
    let address = address_helper();
    let mut f = branching(&gate, &address);
    let before = f.graph.clone();
    let graphs = HashMap::from_iter([
        (gate.graph.symbol.clone(), gate.graph),
        (address.graph.symbol.clone(), address.graph),
    ]);
    let stats = inline_calls_with_cost(
        &mut f.graph,
        &graphs,
        InlineBudget::default(),
        InlineCostPolicy::Observe,
        &GenericCostModel,
    )
    .unwrap();
    let ordered: Vec<_> = stats
        .decisions
        .iter()
        .filter(|decision| matches!(decision.symbol.as_str(), "address_helper" | "scalar_gate"))
        .map(|decision| decision.symbol.as_str())
        .collect();
    assert_eq!(ordered.len(), 10);
    assert!(
        ordered[..9]
            .iter()
            .all(|&symbol| symbol == "address_helper")
    );
    assert_eq!(ordered[9], "scalar_gate");
    assert_eq!(stats.preparation.priority_node_visits, 0);
    f.verify();
    for tag in [32, 2048] {
        equivalent(
            &before,
            &f.graph,
            &graphs,
            &[Value::Scalar(7), Value::Scalar(tag)],
        );
    }
}

#[test]
fn one_frame_can_pass_256_allocations_while_all_frames_share_the_hard_fuel() {
    let leaf = leaf();
    let source = fanout("large_fanout", &leaf.graph, 30);
    let graphs = HashMap::from_iter([
        (source.graph.symbol.clone(), source.graph.clone()),
        (leaf.graph.symbol.clone(), leaf.graph),
    ]);
    let mut f = caller(&source.graph, Some(32), 1);
    let before = f.graph.clone();
    let stats = inline_calls(&mut f.graph, &graphs, InlineBudget::default()).unwrap();
    let work = &stats.preparation;
    assert_eq!(work.calls_inlined, 30);
    assert_eq!(work.free_calls_inlined, 30);
    assert_eq!(work.costed_calls_inlined, 0);
    assert_eq!(work.cached_calls_inlined, 29);
    assert!(work.splice_nodes_allocated > 256);
    assert!(work.clone_budget_used <= 2000);
    assert_eq!(
        work.clone_budget_used,
        work.source_nodes_cloned
            + work.transaction_nodes_cloned
            + work.splice_nodes_allocated
            + work.fact_reapplication_nodes_allocated
    );
    assert!(
        work.decisions
            .iter()
            .filter(|decision| decision.action == "splice" && decision.rejection.is_none())
            .all(|decision| decision.owner == "large_fanout")
    );
    f.verify();
    equivalent(&before, &f.graph, &graphs, &[Value::Scalar(7)]);
}

#[test]
fn zero_and_extreme_clone_budgets_keep_priority_analysis_bounded() {
    let gate = scalar_gate();
    let address = address_helper();
    let source = branching(&gate, &address);
    let graphs = HashMap::from_iter([
        (source.graph.symbol.clone(), source.graph.clone()),
        (gate.graph.symbol.clone(), gate.graph),
        (address.graph.symbol.clone(), address.graph),
    ]);
    for max_nodes in [0, usize::MAX] {
        let mut f = caller(&source.graph, Some(32), 1);
        let before = f.graph.clone();
        let stats = inline_calls(
            &mut f.graph,
            &graphs,
            InlineBudget {
                max_nodes,
                ..InlineBudget::default()
            },
        )
        .unwrap();
        assert_eq!(
            stats.preparation.priority_visit_budget,
            max_nodes.saturating_mul(4)
        );
        assert!(stats.preparation.priority_node_visits <= stats.preparation.priority_visit_budget);
        assert_eq!(
            stats.preparation.fact_reapplication_visit_budget,
            max_nodes.saturating_mul(4)
        );
        assert!(
            stats.preparation.fact_reapplication_node_visits
                <= stats.preparation.fact_reapplication_visit_budget
        );
        assert!(stats.preparation.clone_budget_used <= max_nodes);
        if max_nodes == 0 {
            assert_eq!(stats.preparation.priority_node_visits, 0);
            assert_eq!(stats.preparation.calls_inlined, 0);
            assert_eq!(f.graph.dump(), before.dump());
        } else {
            assert!(stats.preparation.priority_node_visits > 0);
            assert!(calls(&f.graph).is_empty());
        }
        f.verify();
        equivalent(&before, &f.graph, &graphs, &[Value::Scalar(7)]);
    }
}
