//! Real multi-callee constant propagation, compared with executing source
//! calls in shared semantic memory. Preparation accounting is separate from
//! the emitted caller's growth and its NodeIds.
#[path = "semantic_inline_preparation/fixture.rs"]
mod fixture;
#[path = "semantic_inline_preparation/reapplication.rs"]
mod reapplication;
#[path = "semantic_inline_preparation/scheduling.rs"]
mod scheduling;
use fixture::*;
use kfrc_body::fx_hash::HashMap;
use kfrc_son::analysis::cost::GenericCostModel;
use kfrc_son::{
    GraphProvider, InlineBudget, InlineCostPolicy, InlineSkip, Opcode, Value, inline_calls,
    inline_calls_with_cost,
};

#[test]
fn ordinary_splice_constants_reach_the_next_call_before_its_size_decision() {
    let (mut graphs, source) = chain(1);
    let mut increment = Function::new("increment", 1);
    let one = increment.constant(1);
    let result = increment.node(Opcode::Add, &[increment.args[0], one], 0, increment.word);
    increment.ret(result);
    increment.verify();
    let mut f = Function::new("consecutive_calls", 1);
    let thirty_one = f.constant(31);
    let tag = f.call(&increment.graph, &[thirty_one]);
    let result = f.call(&source, &[f.args[0], tag]);
    f.ret(result);
    f.verify();
    graphs.insert(increment.graph.symbol.clone(), increment.graph);
    let before = f.graph.clone();
    let stats = inline_calls(&mut f.graph, &graphs, InlineBudget::default()).unwrap();
    f.verify();
    assert!(calls(&f.graph).is_empty(), "{:?}", stats.skipped);
    assert!(stats.specializations_prepared > 0);
    assert!(stats.seam_node_visits > 0);
    for value in [0, 1, 255, u64::MAX]
        .map(Value::Scalar)
        .into_iter()
        .chain([Value::Poison])
    {
        equivalent(&before, &f.graph, &graphs, &[value]);
    }
}

#[test]
fn repeated_ordinary_splices_share_an_index_and_preserve_poison_observations() {
    let mut increment = Function::new("increment", 1);
    let one = increment.constant(1);
    let result = increment.node(Opcode::Add, &[increment.args[0], one], 0, increment.word);
    increment.ret(result);
    increment.verify();
    let mut f = Function::new("many_seams", 1);
    let mut value = f.args[0];
    for _ in 0..32 {
        value = f.call(&increment.graph, &[value]);
    }
    f.ret(value);
    f.verify();
    let graphs = HashMap::from_iter([(increment.graph.symbol.clone(), increment.graph)]);
    let before = f.graph.clone();
    let stats = inline_calls(&mut f.graph, &graphs, InlineBudget::default()).unwrap();
    f.verify();
    assert_eq!(stats.calls_inlined, 32);
    assert_eq!(stats.seam_control_cleanups, 0);
    // Index work grows with the input plus copied nodes, not 32 full callers.
    assert!(stats.seam_nodes_indexed < 4 * (before.len() + stats.nodes_added));
    for value in [0, 1, 255, u64::MAX]
        .map(Value::Scalar)
        .into_iter()
        .chain([Value::Poison])
    {
        equivalent(&before, &f.graph, &graphs, &[value]);
    }
}

#[test]
fn scalar_seam_allocations_respect_adjacent_hard_growth_limits() {
    let mut increment = Function::new("increment", 1);
    let one = increment.constant(1);
    let result = increment.node(Opcode::Add, &[increment.args[0], one], 0, increment.word);
    increment.ret(result);
    increment.verify();
    let mut f = Function::new("growth_edges", 1);
    let seven = f.constant(7);
    let first = f.call(&increment.graph, &[seven]);
    let result = f.node(Opcode::Add, &[first, f.args[0]], 0, f.word);
    f.ret(result);
    f.verify();
    let before = f.graph;
    let graphs = HashMap::from_iter([(increment.graph.symbol.clone(), increment.graph)]);
    for max_nodes in 0..40 {
        let mut after = before.clone();
        let stats = inline_calls(
            &mut after,
            &graphs,
            InlineBudget {
                max_nodes,
                ..InlineBudget::default()
            },
        )
        .unwrap();
        assert!(
            stats.nodes_added <= max_nodes,
            "budget {max_nodes}: {stats:?}"
        );
        kfrc_son::verify_graph(&mut after).unwrap();
        for value in [Value::Scalar(19), Value::Scalar(u64::MAX), Value::Poison] {
            equivalent(&before, &after, &graphs, &[value]);
        }
    }
}

#[test]
fn constants_cross_multiple_large_callees_before_the_original_wrapper_is_priced() {
    let (graphs, source) = chain(3);
    for policy in [InlineCostPolicy::Legacy, InlineCostPolicy::Observe] {
        let mut f = caller(&source, Some(32), 1);
        let before = f.graph.clone();
        let stats = inline_calls_with_cost(
            &mut f.graph,
            &graphs,
            InlineBudget::default(),
            policy,
            &GenericCostModel,
        )
        .unwrap();
        f.verify();
        assert!(calls(&f.graph).is_empty(), "{:?}", stats.skipped);
        assert!(stats.specializations_prepared >= 3);
        assert!(stats.preparation.calls_inlined >= 3);
        assert!(stats.preparation.cleanup_branches_folded >= 3);
        assert!(stats.preparation.cleanup_nodes_indexed > 0);
        assert!(stats.preparation.cleanup_node_visits > 0);
        assert!(stats.preparation.maximum_depth <= 6);
        assert!(stats.decisions.iter().all(|decision| decision.depth < 4));
        assert!(
            stats
                .preparation
                .decisions
                .iter()
                .all(|decision| decision.symbol != "cold")
        );
        for value in [0, 1, 255, u64::MAX] {
            equivalent(&before, &f.graph, &graphs, &[Value::Scalar(value)]);
        }
        assert!(f.graph.data_objects.is_empty());
    }
}

#[test]
fn dynamic_and_non_ascii_paths_keep_their_original_effects_and_identities() {
    let (graphs, source) = chain(3);
    let mut dynamic = caller(&source, None, 1);
    let before = dynamic.graph.clone();
    let stats = inline_calls(&mut dynamic.graph, &graphs, InlineBudget::default()).unwrap();
    assert_eq!(stats.specializations_prepared, 0);
    for tag in [0, 32, 127, 128, 2048, 0x10ffff] {
        equivalent(
            &before,
            &dynamic.graph,
            &graphs,
            &[Value::Scalar(19), Value::Scalar(tag)],
        );
    }
    let mut non_ascii = caller(&source, Some(2048), 1);
    let before = non_ascii.graph.clone();
    let stats = inline_calls(&mut non_ascii.graph, &graphs, InlineBudget::default()).unwrap();
    assert!(
        stats
            .skipped
            .iter()
            .any(|(_, skip)| *skip == InlineSkip::DataConflict)
    );
    equivalent(&before, &non_ascii.graph, &graphs, &[Value::Scalar(19)]);
}

#[test]
fn a_single_nonrefundable_account_covers_sources_transactions_and_splices() {
    let (mut graphs, mut source) = chain(4);
    // Dead storage is copied by FunctionGraph::clone too, so it must remain
    // charged even though live-node pricing cannot see it.
    let word = source
        .rust_type(source.find_projection(source.start, 2).unwrap())
        .unwrap();
    let dead = source.add_constant(999, word);
    source.remove_node(dead);
    graphs.insert(source.symbol.clone(), source.clone());
    assert!(source.len() > source.live_count());
    for nodes in [source.len() - 1, source.len() + 1, 700, 2000] {
        let mut f = caller(&source, Some(32), 3);
        let before = f.graph.clone();
        let budget = InlineBudget {
            max_nodes: nodes,
            ..InlineBudget::default()
        };
        let stats = inline_calls(&mut f.graph, &graphs, budget).unwrap();
        let work = &stats.preparation;
        assert!(work.clone_budget_used <= nodes);
        assert!(work.priority_node_visits <= nodes.saturating_mul(4));
        assert!(work.costed_calls_inlined <= 8);
        assert_eq!(
            work.calls_inlined,
            work.costed_calls_inlined + work.free_calls_inlined
        );
        assert_eq!(work.cache_hits, stats.specialization_cache_hits);
        assert_eq!(
            work.clone_budget_used,
            work.source_nodes_cloned
                + work.transaction_nodes_cloned
                + work.splice_nodes_allocated
                + work.fact_reapplication_nodes_allocated
        );
        assert_eq!(
            stats.specialization_nodes_cloned,
            work.source_nodes_cloned + work.transaction_nodes_cloned + work.splice_nodes_cloned
        );
        assert!(stats.specialization_nodes_cloned <= work.clone_budget_used);
        if nodes < source.len() {
            assert_eq!(work.source_nodes_cloned, 0);
        }
        f.verify();
        equivalent(&before, &f.graph, &graphs, &[Value::Scalar(7)]);
    }
}

#[test]
fn deep_preparation_is_bounded_and_a_shallow_call_does_not_reuse_its_depth_limit() {
    let (graphs, source) = chain(7);
    let direct = &graphs["wrapper_5"];
    let mut f = caller(&source, Some(32), 1);
    // Retire the first Return, then call the same inner callee from the outer
    // graph with more preparation depth available and the same scalar facts.
    let returned = f
        .graph
        .ids()
        .find(|&node| f.graph.op(node) == Opcode::Return)
        .unwrap();
    let first = f.graph.input(returned, 1);
    f.graph.remove_node(returned);
    let tag = f.constant(32);
    let result = f.call(direct, &[first, tag]);
    f.ret(result);
    f.verify();
    let before = f.graph.clone();
    let stats = inline_calls(
        &mut f.graph,
        &graphs,
        InlineBudget {
            max_nodes: 6000,
            ..InlineBudget::default()
        },
    )
    .unwrap();
    assert!(stats.preparation.maximum_depth <= 6);
    assert!(
        stats
            .preparation
            .decisions
            .iter()
            .any(|decision| decision.depth == 6 && decision.rejection == Some(InlineSkip::Budget))
    );
    assert!(
        stats
            .preparation
            .decisions
            .iter()
            .any(|decision| decision.symbol == "wrapper_5"
                && decision.depth == 0
                && decision.action == "prepared")
    );
    assert!(stats.preparation.costed_calls_inlined <= 8);
    f.verify();
    equivalent(&before, &f.graph, &graphs, &[Value::Scalar(7)]);
}

#[test]
fn cached_candidates_remain_usable_after_preparation_fuel_is_spent() {
    let (graphs, source) = chain(1);
    let mut probe = caller(&source, Some(32), 1);
    let work = inline_calls(&mut probe.graph, &graphs, InlineBudget::default())
        .unwrap()
        .preparation
        .clone_budget_used;
    let mut f = caller(&source, Some(32), 2);
    let before = f.graph.clone();
    let budget = InlineBudget {
        max_nodes: work,
        ..InlineBudget::default()
    };
    let stats = inline_calls(&mut f.graph, &graphs, budget).unwrap();
    assert_eq!(stats.preparation.clone_budget_used, work);
    assert!(stats.specialization_cache_hits > 0);
    assert!(stats.calls_inlined >= 2);
    f.verify();
    equivalent(&before, &f.graph, &graphs, &[Value::Scalar(7)]);
}

#[test]
fn constant_when_defined_arguments_are_observed_before_any_body_effect() {
    let (graphs, source) = chain(3);
    let mut f = Function::new("poisoned", 1);
    let poison = f.node(Opcode::Undefined, &[], 0, f.word);
    let zero = f.constant(0);
    let tag = f.constant(32);
    let masked = f.node(Opcode::BitAnd, &[poison, zero], 0, f.word);
    let known = f.node(Opcode::BitOr, &[masked, tag], 0, f.word);
    let result = f.call(&source, &[f.args[0], known]);
    f.ret(result);
    f.verify();
    let before = f.graph.clone();
    let stats = inline_calls(&mut f.graph, &graphs, InlineBudget::default()).unwrap();
    assert!(stats.calls_inlined > 0);
    f.verify();
    equivalent(&before, &f.graph, &graphs, &[Value::Scalar(7)]);
    let result = execute(&f.graph, &graphs, &[Value::Scalar(7)]);
    assert!(matches!(
        result.0,
        Err(kfrc_son::InterpretError::PoisonObserved(_))
    ));
    assert!(result.1.is_empty());
}

#[test]
fn noreturn_preparation_publishes_a_terminal_seam() {
    let (mut graphs, source) = chain(1);
    let mut source = source;
    let hot = source
        .ids()
        .find(|&node| {
            source.op(node) == Opcode::Return
                && source.op(source.input(node, 1)) == Opcode::Projection
        })
        .unwrap();
    source.set_opcode(hot, Opcode::Unreachable);
    source.set_inputs(hot, &[]);
    kfrc_son::verify_graph(&mut source).unwrap();
    graphs.insert(source.symbol.clone(), source.clone());
    let mut f = caller(&source, Some(32), 2);
    let before = f.graph.clone();
    let stats = inline_calls(&mut f.graph, &graphs, InlineBudget::default()).unwrap();
    assert!(stats.calls_inlined > 0);
    assert!(stats.specializations_prepared > 0);
    assert!(stats.specialization_nodes_cloned > 0);
    assert!(!f.graph.ids().any(|node| f.graph.op(node) == Opcode::Return));
    assert!(
        !calls(&f.graph)
            .iter()
            .any(|symbol| symbol == &source.symbol)
    );
    for argument in [Value::Scalar(7), Value::Scalar(u64::MAX)] {
        let original = execute(&before, &graphs, &[argument.clone()]);
        let optimized = execute(&f.graph, &graphs, &[argument]);
        assert_eq!(original.1, optimized.1);
        assert!(matches!(
            original.0,
            Err(kfrc_son::InterpretError::UndefinedBehaviour(_))
        ));
        assert_eq!(optimized.0, Ok(kfrc_son::SemanticOutcome::Unreachable));
    }
}

struct PrivateProvider {
    graphs: HashMap<String, kfrc_son::FunctionGraph>,
    deny_private: bool,
}
impl GraphProvider for PrivateProvider {
    fn graph(&self, symbol: &str) -> Option<&kfrc_son::FunctionGraph> {
        self.graphs.get(symbol)
    }
    fn can_reference(&self, symbol: &str) -> bool {
        !self.deny_private || symbol != "foreign_private"
    }
}

#[test]
fn surviving_private_calls_and_identity_data_are_checked_by_the_outer_plan() {
    for identity in [false, true] {
        let mut target = leaf();
        target.graph.symbol = "foreign_private".into();
        let mut source = guarded("outer", &target.graph);
        if identity {
            let value = source
                .graph
                .ids()
                .find(|&node| {
                    source.graph.op(node) == Opcode::Call
                        && source.graph.callees[source.graph.payload(node) as usize].symbol
                            == "foreign_private"
                })
                .unwrap();
            let saved_control = source.control;
            let saved_effect = source.effect;
            source.control = source.graph.control(value);
            source.effect = source.graph.input(value, 0);
            let address = source.address("live_identity", true);
            source.graph.set_input(value, 1, address);
            source.control = saved_control;
            source.effect = saved_effect;
        }
        source.verify();
        let provider = PrivateProvider {
            graphs: HashMap::from_iter([(source.graph.symbol.clone(), source.graph.clone())]),
            deny_private: !identity,
        };
        let mut f = caller(&source.graph, Some(32), 1);
        let before = f.graph.dump();
        let stats = inline_calls(&mut f.graph, &provider, InlineBudget::default()).unwrap();
        assert_eq!(stats.calls_inlined, 0);
        let expected = if identity {
            InlineSkip::DataConflict
        } else {
            InlineSkip::PrivateReference
        };
        assert!(stats.skipped.iter().any(|(_, skip)| *skip == expected));
        assert_eq!(f.graph.dump(), before);
        assert!(f.graph.data_objects.is_empty());
    }
}

#[test]
fn source_recursion_and_abi_mismatch_do_not_gain_permission_from_constants() {
    let mut target = leaf();
    target.graph.symbol = "recursive".into();
    let mut recursive = guarded("recursive", &target.graph);
    let provider = HashMap::from_iter([(recursive.graph.symbol.clone(), recursive.graph.clone())]);
    let mut f = caller(&recursive.graph, Some(32), 1);
    let before = f.graph.dump();
    let stats = inline_calls(&mut f.graph, &provider, InlineBudget::default()).unwrap();
    assert!(
        stats
            .skipped
            .iter()
            .any(|(_, skip)| *skip == InlineSkip::Recursive)
    );
    assert_eq!(f.graph.dump(), before);
    recursive.graph.abi.c_variadic = true;
    let provider = HashMap::from_iter([(recursive.graph.symbol.clone(), recursive.graph)]);
    let mut f = caller(&target.graph, Some(32), 1);
    let before = f.graph.dump();
    let stats = inline_calls(&mut f.graph, &provider, InlineBudget::default()).unwrap();
    assert!(
        stats
            .skipped
            .iter()
            .any(|(_, skip)| *skip == InlineSkip::AbiMismatch)
    );
    assert_eq!(f.graph.dump(), before);
}

#[test]
fn sibling_preparation_trees_share_eight_costed_helpers() {
    // Each two-call group is independently within the scratch growth limit.
    // Their parents must share the helper cap rather than reset it per group.
    let helper = costed_helper();
    let mut graphs = HashMap::from_iter([(helper.graph.symbol.clone(), helper.graph.clone())]);
    let groups: Vec<_> = (0..3)
        .map(|i| {
            let group = fanout(&format!("group_{i}"), &helper.graph, 2);
            graphs.insert(group.graph.symbol.clone(), group.graph.clone());
            group.graph
        })
        .collect();
    let mut source = guarded("fanout", &groups[0]);
    let (mut value, hot) = open_hot_return(&mut source);
    for group in &groups[1..] {
        value = source.call(group, &[value, source.args[1]]);
    }
    source.ret(value);
    source.verify();
    assert!(source.graph.is_dead(hot));
    graphs.insert(source.graph.symbol.clone(), source.graph.clone());
    let mut f = caller(&source.graph, Some(32), 1);
    let before = f.graph.clone();
    let stats = inline_calls(&mut f.graph, &graphs, InlineBudget::default()).unwrap();
    assert_eq!(stats.preparation.costed_calls_inlined, 8);
    assert_eq!(stats.preparation.free_calls_inlined, 0);
    assert_eq!(stats.preparation.calls_inlined, 8);
    assert!(stats.preparation.budget_bailouts > 0);
    assert!(
        stats
            .preparation
            .decisions
            .iter()
            .any(|decision| decision.symbol == "group_2"
                && decision.action == "splice"
                && decision.rejection == Some(InlineSkip::Budget))
    );
    f.verify();
    equivalent(&before, &f.graph, &graphs, &[Value::Scalar(7)]);
}

#[test]
fn free_cached_helpers_can_exceed_eight_but_still_exhaust_shared_clone_fuel() {
    let leaf = leaf();
    let source = fanout("free_fanout", &leaf.graph, 14);
    let graphs = HashMap::from_iter([
        (source.graph.symbol.clone(), source.graph.clone()),
        (leaf.graph.symbol.clone(), leaf.graph),
    ]);
    let mut probe = caller(&source.graph, Some(32), 1);
    let complete = inline_calls(&mut probe.graph, &graphs, InlineBudget::default()).unwrap();
    assert_eq!(complete.preparation.calls_inlined, 14);
    assert_eq!(complete.preparation.free_calls_inlined, 14);
    assert_eq!(complete.preparation.costed_calls_inlined, 0);
    assert_eq!(complete.preparation.cached_calls_inlined, 13);
    assert!(complete.preparation.cached_splice_nodes_allocated > 0);
    let fuel = complete.preparation.clone_budget_used - 1;
    let mut f = caller(&source.graph, Some(32), 1);
    let before = f.graph.clone();
    let stats = inline_calls(
        &mut f.graph,
        &graphs,
        InlineBudget {
            max_nodes: fuel,
            ..InlineBudget::default()
        },
    )
    .unwrap();
    assert!(stats.preparation.clone_budget_used <= fuel);
    assert!(stats.preparation.free_calls_inlined < 14);
    assert_eq!(stats.preparation.costed_calls_inlined, 0);
    assert!(stats.preparation.cached_calls_inlined > 0);
    assert!(stats.preparation.cached_splice_nodes_allocated > 0);
    assert!(stats.specialization_cache_hits > 0);
    assert!(stats.preparation.budget_bailouts > 0);
    assert_eq!(
        stats.preparation.clone_budget_used,
        stats.preparation.source_nodes_cloned
            + stats.preparation.transaction_nodes_cloned
            + stats.preparation.splice_nodes_allocated
            + stats.preparation.fact_reapplication_nodes_allocated
    );
    f.verify();
    equivalent(&before, &f.graph, &graphs, &[Value::Scalar(7)]);
}
