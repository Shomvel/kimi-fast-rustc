#[allow(dead_code)]
#[path = "../../../../kfrc-son/tests/semantic_inline_preparation/fixture.rs"]
mod fixture;

use super::{MAX_WORK, PreparedGraphs, emission_needs_retry, run_with_configuration, schedule};
use crate::codegen::{CrateGraphs, CrateProvider};
use fixture::{Function, calls, equivalent};
use kfrc_body::fx_hash::HashMap;
use kfrc_body::{DataObject, DataRelocation};
use kfrc_son::{
    FunctionGraph, GraphProvider, InlineBudget, InlineSkip, InlineStats, Opcode, Value,
};
use std::collections::BTreeSet;

fn leaf(name: &str) -> FunctionGraph {
    let mut function = Function::new(name, 1);
    let one = function.constant(1);
    let value = function.node(Opcode::BitXor, &[function.args[0], one], 0, function.word);
    function.ret(value);
    function.verify();
    function.graph
}

fn wrapper(name: &str, target: &FunctionGraph) -> FunctionGraph {
    let mut function = Function::new(name, 1);
    let value = function.call(target, &function.args.clone());
    function.ret(value);
    function.verify();
    function.graph
}

fn arithmetic_leaf(name: &str) -> FunctionGraph {
    let mut function = Function::new(name, 1);
    let one = function.constant(1);
    let mut value = function.args[0];
    for _ in 0..16 {
        value = function.node(Opcode::Mul, &[value, value], 0, function.word);
        value = function.node(Opcode::Add, &[value, one], 0, function.word);
    }
    function.ret(value);
    function.verify();
    assert!(function.graph.live_count() > 24 && function.graph.live_count() <= 64);
    function.graph
}

fn optimize(graphs: &mut CrateGraphs, roots: &[&str], budget: usize) -> super::report::Summary {
    optimize_with_policy(graphs, roots, budget, kfrc_son::InlineCostPolicy::Legacy)
}

fn optimize_with_policy(
    graphs: &mut CrateGraphs,
    roots: &[&str],
    budget: usize,
    policy: kfrc_son::InlineCostPolicy,
) -> super::report::Summary {
    graphs.collect_references();
    let roots = roots.iter().map(|name| (*name).to_owned()).collect();
    run_with_configuration(
        graphs,
        &roots,
        budget,
        policy,
        &crate::inline_cost::Limits::default(),
    )
    .unwrap_or_else(|error| panic!("{}: {}", error.symbol, error.message))
}

fn inline_emitted_copy(graphs: &CrateGraphs, symbol: &str) -> FunctionGraph {
    inline_emitted_copy_with_policy(graphs, symbol, kfrc_son::InlineCostPolicy::Legacy)
}

fn inline_emitted_copy_with_policy(
    graphs: &CrateGraphs,
    symbol: &str,
    policy: kfrc_son::InlineCostPolicy,
) -> FunctionGraph {
    let provider = CrateProvider { all: graphs };
    let start = provider.emission_graph(symbol);
    let mut emitted = start.graph.clone();
    if start.inlining_complete {
        return emitted;
    }
    let inlined = kfrc_son::inline_calls_with_cost(
        &mut emitted,
        &provider,
        InlineBudget {
            max_nodes: super::MAX_INLINE_NODES,
            ..InlineBudget::default()
        },
        policy,
        crate::inline_cost::model(policy),
    )
    .unwrap();
    kfrc_son::optimize_specialized_seams(
        &mut emitted,
        &inlined.dirty_nodes,
        &inlined.argument_slots,
    )
    .unwrap();
    emitted
}

#[path = "emission_tests.rs"]
mod emission;

#[test]
fn long_wrapper_chain_publishes_callees_before_callers() {
    let mut graphs = CrateGraphs::default();
    let mut child = leaf("z_leaf");
    graphs.bodies.insert(child.symbol.clone(), child.clone());
    for index in (0..12).rev() {
        let parent = wrapper(&format!("wrapper_{index:02}"), &child);
        graphs.bodies.insert(parent.symbol.clone(), parent.clone());
        child = parent;
    }
    let originals = HashMap::from_iter(graphs.bodies.clone());
    let before = child;
    let summary = optimize(&mut graphs, &["wrapper_00"], MAX_WORK);

    assert_eq!(summary.published, 12);
    assert_eq!(summary.reachable_functions, 13);
    assert_eq!(summary.final_calls, 0);
    assert_eq!(graphs.inlining_complete.len(), 13);
    for graph in graphs.bodies.values() {
        assert!(calls(graph).is_empty());
        assert!(graph.live_count() <= 24);
    }
    for argument in [Value::Scalar(0), Value::Scalar(u64::MAX), Value::Poison] {
        equivalent(
            &before,
            &graphs.bodies["wrapper_00"],
            &originals,
            &[argument],
        );
    }
}

#[test]
fn diamond_reuses_published_source_and_has_stable_order() {
    let leaf = leaf("leaf");
    let left = wrapper("left", &leaf);
    let right = wrapper("right", &leaf);
    let mut root = Function::new("root", 1);
    let first = root.call(&left, &root.args.clone());
    let second = root.call(&right, &root.args.clone());
    let sum = root.node(Opcode::Add, &[first, second], 0, root.word);
    root.ret(sum);
    root.verify();
    let before = root.graph.clone();
    let mut graphs = CrateGraphs::default();
    for graph in [root.graph, right, leaf, left] {
        graphs.bodies.insert(graph.symbol.clone(), graph);
    }
    let originals = HashMap::from_iter(graphs.bodies.clone());
    let mut repeated = CrateGraphs::default();
    for (symbol, graph) in graphs.bodies.iter().rev() {
        repeated.bodies.insert(symbol.clone(), graph.clone());
    }
    let summary = optimize(&mut graphs, &["root"], MAX_WORK);
    let again = optimize(&mut repeated, &["root"], MAX_WORK);

    assert_eq!(summary.published, 3);
    assert_eq!(summary.calls_inlined, 4);
    assert_eq!(summary.work_used, again.work_used);
    assert_eq!(summary.final_calls, 0);
    for (symbol, graph) in &graphs.bodies {
        assert_eq!(graph.dump(), repeated.bodies[symbol].dump());
    }
    for argument in [Value::Scalar(7), Value::Scalar(u64::MAX), Value::Poison] {
        equivalent(&before, &graphs.bodies["root"], &originals, &[argument]);
    }
}

#[test]
fn recursive_components_and_self_calls_stay_on_the_existing_path() {
    let mut first = Function::new("first", 1);
    let abi = first.graph.abi.clone();
    let result = first.call_named("second", &abi, &first.args.clone());
    first.ret(result);
    first.verify();
    let second = wrapper("second", &first.graph);
    let parent = wrapper("parent", &first.graph);
    let mut self_call = Function::new("self_call", 1);
    let result = self_call.call_named("self_call", &abi, &self_call.args.clone());
    self_call.ret(result);
    self_call.verify();
    let mut graphs = CrateGraphs::default();
    for graph in [first.graph, second, parent, self_call.graph] {
        graphs.bodies.insert(graph.symbol.clone(), graph);
    }
    let originals = graphs.bodies.clone();
    let summary = optimize(&mut graphs, &["parent", "self_call"], MAX_WORK);

    assert_eq!(summary.recursive, 3);
    assert_eq!(summary.published, 0);
    assert!(graphs.inlining_complete.is_empty());
    for (symbol, before) in originals {
        assert_eq!(before.dump(), graphs.bodies[&symbol].dump());
    }
    // Same-SCC sources must be hidden even from the identity-summary query.
    let ready = BTreeSet::new();
    let provider = PreparedGraphs {
        graphs: &graphs,
        ready: &ready,
    };
    assert!(provider.graph("first").is_none());
    assert!(provider.preparation_graph("second").is_none());
}

#[test]
fn noinline_missing_and_unready_sources_have_distinct_emission_fallbacks() {
    let blocked = leaf("blocked");
    let parent = wrapper("parent", &blocked);
    let mut graphs = CrateGraphs::default();
    graphs
        .bodies
        .extend([("blocked".into(), blocked), ("parent".into(), parent)]);
    graphs.no_inline.insert("blocked".into());
    graphs.always_inline.insert("blocked".into());
    optimize(&mut graphs, &["parent"], MAX_WORK);
    assert_eq!(calls(&graphs.bodies["parent"]), ["blocked"]);
    assert!(graphs.inlining_complete.contains("parent"));
    let ready = BTreeSet::from(["blocked".into()]);
    let provider = PreparedGraphs {
        graphs: &graphs,
        ready: &ready,
    };
    assert!(provider.always_inline("blocked"));
    assert!(provider.graph("blocked").is_none());
    assert!(provider.preparation_graph("blocked").is_none());

    let external = leaf("external");
    let caller = wrapper("caller", &external);
    let mut unavailable = CrateGraphs::default();
    unavailable.bodies.insert("caller".into(), caller.clone());
    optimize(&mut unavailable, &["caller"], MAX_WORK);
    assert!(unavailable.inlining_complete.contains("caller"));
    unavailable.bodies.insert("external".into(), external);
    let ready = BTreeSet::new();
    assert!(emission_needs_retry(
        &caller,
        &PreparedGraphs {
            graphs: &unavailable,
            ready: &ready
        },
        &InlineStats::default(),
    ));
}

#[test]
fn publication_exposes_private_references_to_every_cgu() {
    let private = leaf("private");
    let nested = wrapper("nested", &private);
    let root = wrapper("root", &nested);
    let mut graphs = CrateGraphs::default();
    graphs.bodies.extend([
        (private.symbol.clone(), private),
        (nested.symbol.clone(), nested),
        (root.symbol.clone(), root.clone()),
    ]);
    graphs.internal.extend(["private".into(), "nested".into()]);
    graphs.no_inline.insert("private".into());
    let summary = optimize(&mut graphs, &["root"], MAX_WORK);

    assert_eq!(summary.private_reference, 0);
    assert_ne!(graphs.bodies["root"].dump(), root.dump());
    assert_eq!(calls(&graphs.bodies["root"]), ["private"]);
    assert!(graphs.inlining_complete.contains("root"));
    assert!(CrateProvider { all: &graphs }.graph("nested").is_some());
    assert_eq!(calls(&graphs.bodies["nested"]), ["private"]);
}

#[test]
fn reachability_includes_data_relocations_and_budget_never_changes_fallback_sources() {
    let target = leaf("data_target");
    let ignored = wrapper("unreachable", &target);
    let mut root = leaf("root");
    root.data_objects.push(DataObject {
        symbol: "table".into(),
        bytes: vec![0; 8],
        align: 8,
        relocations: vec![DataRelocation {
            offset: 0,
            target: "data_target".into(),
            addend: 0,
        }],
        ..Default::default()
    });
    let mut graphs = CrateGraphs::default();
    graphs.bodies.extend([
        (root.symbol.clone(), root),
        (target.symbol.clone(), target),
        (ignored.symbol.clone(), ignored.clone()),
    ]);
    let summary = optimize(&mut graphs, &["root"], MAX_WORK);
    assert_eq!(summary.reachable_functions, 2);
    assert!(!graphs.inlining_complete.contains("unreachable"));
    assert_eq!(graphs.bodies["unreachable"].dump(), ignored.dump());

    let child = leaf("child");
    let parent = wrapper("parent", &child);
    let mut limited = CrateGraphs::default();
    limited
        .bodies
        .extend([("child".into(), child), ("parent".into(), parent.clone())]);
    let summary = optimize(&mut limited, &["parent"], 0);
    assert_eq!(summary.work_used, 0);
    assert_eq!(summary.work_budget, 1);
    assert!(!limited.inlining_complete.contains("parent"));
    assert_eq!(limited.bodies["parent"].dump(), parent.dump());
}

#[test]
fn prepared_source_is_shared_by_pricing_and_emission() {
    let helper = fixture::costed_helper().graph;
    let parent = wrapper("parent", &helper);
    assert!(parent.live_count() <= 24);
    let mut graphs = CrateGraphs::default();
    graphs.bodies.extend([
        (helper.symbol.clone(), helper),
        (parent.symbol.clone(), parent.clone()),
    ]);
    let summary = optimize(&mut graphs, &["parent"], MAX_WORK);
    assert_eq!(summary.profitability_growth, 0);
    assert_ne!(graphs.bodies["parent"].dump(), parent.dump());
    assert!(
        !calls(&graphs.bodies["parent"])
            .iter()
            .any(|s| s == "helper")
    );
    assert!(graphs.inlining_complete.contains("parent"));
}

#[test]
fn small_call_free_publication_preserves_depth_for_a_long_wrapper_chain() {
    let mut graphs = CrateGraphs::default();
    let mut child = arithmetic_leaf("arithmetic");
    graphs.bodies.insert(child.symbol.clone(), child.clone());
    for index in (0..8).rev() {
        let parent = wrapper(&format!("wrapper_{index}"), &child);
        assert!(parent.live_count() <= 24);
        graphs.bodies.insert(parent.symbol.clone(), parent.clone());
        child = parent;
    }
    let originals = HashMap::from_iter(graphs.bodies.clone());
    let before = child;
    // Keeping every growing wrapper at its original size leaves this chain
    // longer than the ordinary emission depth, even with ample node fuel.
    let mut unprepared = before.clone();
    let ordinary = kfrc_son::inline_calls(
        &mut unprepared,
        &originals,
        InlineBudget {
            max_nodes: super::MAX_INLINE_NODES,
            ..InlineBudget::default()
        },
    )
    .unwrap();
    assert!(
        ordinary
            .skipped
            .iter()
            .any(|(_, reason)| *reason == InlineSkip::Budget)
    );
    assert!(!calls(&unprepared).is_empty());

    let summary = optimize(&mut graphs, &["wrapper_0"], MAX_WORK);
    assert_eq!(summary.published, 8);
    assert_eq!(summary.profitability_growth, 0);
    for graph in graphs.bodies.values() {
        assert!(calls(graph).is_empty());
        assert!(graph.live_count() > 24 && graph.live_count() <= 64);
        assert!(graphs.inlining_complete.contains(&graph.symbol));
    }
    for argument in [Value::Scalar(0), Value::Scalar(u64::MAX), Value::Poison] {
        equivalent(
            &before,
            &graphs.bodies["wrapper_0"],
            &originals,
            &[argument],
        );
    }
}

#[test]
fn growing_an_already_large_source_publishes_one_prepared_body() {
    let helper = arithmetic_leaf("arithmetic");
    let mut large = Function::new("large", 1);
    let mut value = large.call(&helper, &large.args.clone());
    let one = large.constant(1);
    for _ in 0..32 {
        value = large.node(Opcode::Mul, &[value, value], 0, large.word);
        value = large.node(Opcode::Add, &[value, one], 0, large.word);
    }
    large.ret(value);
    large.verify();
    let before = large.graph;
    assert!(before.live_count() > 64);
    let mut graphs = CrateGraphs::default();
    graphs.bodies.extend([
        (helper.symbol.clone(), helper),
        (before.symbol.clone(), before.clone()),
    ]);
    let originals = HashMap::from_iter(graphs.bodies.clone());
    let summary = optimize(&mut graphs, &["large"], MAX_WORK);
    assert_eq!(summary.profitability_growth, 0);
    assert_ne!(graphs.bodies["large"].dump(), before.dump());
    assert!(graphs.inlining_complete.contains("large"));

    // Emission starts from the same already prepared graph.
    let emitted = inline_emitted_copy(&graphs, "large");
    assert!(calls(&emitted).is_empty());
    assert!(emitted.live_count() > before.live_count());
    for argument in [Value::Scalar(7), Value::Scalar(u64::MAX), Value::Poison] {
        equivalent(&before, &emitted, &originals, &[argument]);
    }
}

#[test]
fn residual_threshold_controls_growth_without_a_second_emission_source() {
    let helper = arithmetic_leaf("arithmetic");
    let parent = wrapper("parent", &helper);
    let before = parent.clone();
    let originals = HashMap::from_iter([
        (helper.symbol.clone(), helper.clone()),
        (parent.symbol.clone(), parent.clone()),
    ]);
    for (max_growth, should_inline) in [(0, false), (16, false), (40, true)] {
        let policy = kfrc_son::InlineCostPolicy::Residual { max_growth };
        let mut graphs = CrateGraphs::default();
        graphs.bodies.extend([
            (helper.symbol.clone(), helper.clone()),
            (parent.symbol.clone(), parent.clone()),
        ]);
        let summary = optimize_with_policy(&mut graphs, &["parent"], MAX_WORK, policy);
        assert_eq!(summary.published, usize::from(should_inline));
        let emitted = inline_emitted_copy_with_policy(&graphs, "parent", policy);
        assert_eq!(calls(&emitted).is_empty(), should_inline);
        let provider = CrateProvider { all: &graphs };
        assert!(std::ptr::eq(
            provider.graph("parent").unwrap(),
            provider.emission_graph("parent").graph
        ));
        for value in [Value::Scalar(7), Value::Scalar(u64::MAX), Value::Poison] {
            equivalent(&before, &emitted, &originals, &[value]);
        }
    }
}

#[test]
fn caller_uses_the_prepared_source_after_growth() {
    let helper = fixture::costed_helper().graph;
    let retained = wrapper("retained", &helper);
    assert!(retained.live_count() <= 24);
    let mut caller = Function::new("caller", 1);
    let abi = caller.graph.abi.clone();
    let mut value = caller.call(&retained, &caller.args.clone());
    for _ in 0..18 {
        value = caller.call_named("touch", &abi, &[value]);
    }
    caller.ret(value);
    caller.verify();
    assert!(caller.graph.live_count() > 64);
    let before = caller.graph.clone();
    let mut graphs = CrateGraphs::default();
    for graph in [helper, retained.clone(), caller.graph] {
        graphs.bodies.insert(graph.symbol.clone(), graph);
    }
    let originals = HashMap::from_iter(graphs.bodies.clone());
    let summary = optimize(&mut graphs, &["caller"], MAX_WORK);

    assert_eq!(summary.profitability_growth, 0);
    // Prepare helper -> retained once, then splice that prepared body into
    // caller. The former second view repeated helper's splice at the caller.
    assert_eq!(summary.calls_inlined, 2);
    assert_ne!(graphs.bodies["retained"].dump(), retained.dump());
    assert!(graphs.inlining_complete.contains("retained"));
    assert_ne!(graphs.bodies["caller"].dump(), before.dump());
    assert!(graphs.inlining_complete.contains("caller"));
    let emitted = inline_emitted_copy(&graphs, "caller");
    assert_eq!(calls(&emitted), vec!["touch"; 23]);
    for argument in [Value::Scalar(7), Value::Scalar(u64::MAX), Value::Poison] {
        equivalent(&before, &emitted, &originals, &[argument]);
    }
}

#[test]
fn prepared_private_source_is_available_to_all_callers() {
    let private = leaf("private");
    let nested = wrapper("nested", &private);
    let retained = wrapper("retained", &nested);
    let mut caller = Function::new("caller", 1);
    let abi = caller.graph.abi.clone();
    let value = caller.call(&retained, &caller.args.clone());
    // Another direct reference to the same private dependency must survive
    // independently of the prepared retained body.
    let mut value = caller.call(&private, &[value]);
    for _ in 0..18 {
        value = caller.call_named("touch", &abi, &[value]);
    }
    caller.ret(value);
    caller.verify();
    assert!(caller.graph.live_count() > 64);
    let before = caller.graph.clone();
    let mut graphs = CrateGraphs::default();
    for graph in [private, nested, retained.clone(), caller.graph] {
        graphs.bodies.insert(graph.symbol.clone(), graph);
    }
    graphs.internal.extend(["private".into(), "nested".into()]);
    graphs.no_inline.insert("private".into());
    graphs.always_inline.insert("private".into());
    let originals = HashMap::from_iter(graphs.bodies.clone());
    let summary = optimize(&mut graphs, &["caller"], MAX_WORK);

    assert_eq!(summary.private_reference, 0);
    assert_ne!(graphs.bodies["retained"].dump(), retained.dump());
    assert!(graphs.inlining_complete.contains("retained"));
    let emitted = inline_emitted_copy(&graphs, "caller");
    let after = &emitted;
    assert_eq!(
        calls(after)
            .iter()
            .filter(|&name| name == "private")
            .count(),
        2
    );
    assert!(
        !calls(after)
            .iter()
            .any(|name| matches!(name.as_str(), "retained" | "nested"))
    );
    assert!(CrateProvider { all: &graphs }.graph("caller").is_some());
    for argument in [Value::Scalar(7), Value::Scalar(u64::MAX), Value::Poison] {
        equivalent(&before, after, &originals, &[argument]);
    }
}

#[test]
fn shared_scratch_size_rejection_retains_the_cgu_sole_call_opportunity() {
    let mut helper = Function::new("helper", 1);
    let abi = helper.graph.abi.clone();
    let mut value = helper.args[0];
    for _ in 0..18 {
        value = helper.call_named("touch", &abi, &[value]);
    }
    helper.ret(value);
    helper.verify();
    assert!(helper.graph.live_count() > 64);
    let parent = wrapper("parent", &helper.graph);
    let mut graphs = CrateGraphs::default();
    graphs.bodies.extend([
        (helper.graph.symbol.clone(), helper.graph),
        (parent.symbol.clone(), parent.clone()),
    ]);
    graphs.internal.insert("helper".into());
    optimize(&mut graphs, &["parent"], MAX_WORK);
    assert_eq!(calls(&graphs.bodies["parent"]), ["helper"]);
    assert!(!graphs.inlining_complete.contains("parent"));
    assert!(CrateProvider { all: &graphs }.sole_call_site("helper"));
}

#[test]
fn schedule_is_iterative_and_orders_every_nonrecursive_edge_bottom_up() {
    let mut graphs = CrateGraphs::default();
    let mut child = leaf("leaf");
    graphs.bodies.insert(child.symbol.clone(), child.clone());
    for index in 0..2048 {
        let parent = wrapper(&format!("f{index:04}"), &child);
        graphs.bodies.insert(parent.symbol.clone(), parent.clone());
        child = parent;
    }
    let reachable = graphs.bodies.keys().cloned().collect();
    let order = schedule::postorder(&graphs.bodies, &reachable);
    assert_eq!(order.len(), 2049);
    assert_eq!(order[0].members, ["leaf"]);
    for (index, component) in order.iter().skip(1).enumerate() {
        assert!(!component.recursive);
        assert_eq!(component.members, [format!("f{index:04}")]);
    }
}
