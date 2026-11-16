#[allow(dead_code)]
#[path = "semantic_inline_preparation/fixture.rs"]
mod fixture;

use fixture::{Function, caller, calls, chain, equivalent};
use kfrc_body::fx_hash::HashMap;
use kfrc_son::analysis::cost::{ResidualCostModel, estimate_graph};
use kfrc_son::{
    FunctionGraph, InlineBudget, InlineCostPolicy, InlineSkip, Opcode, Value,
    inline_calls_with_cost,
};

fn inline(
    graph: &mut FunctionGraph,
    sources: &HashMap<String, FunctionGraph>,
    budget: InlineBudget,
    max_growth: u64,
) -> kfrc_son::InlineStats {
    inline_calls_with_cost(
        graph,
        sources,
        budget,
        InlineCostPolicy::Residual { max_growth },
        &ResidualCostModel,
    )
    .unwrap()
}

#[test]
fn source_requirements_do_not_hide_a_free_body_behind_a_node_limit() {
    let mut source = Function::new("observations", 1);
    for _ in 0..40 {
        source.observe(source.args[0]);
    }
    source.ret(source.args[0]);
    source.verify();
    assert!(source.graph.live_count() > 64);
    let mut f = Function::new("caller", 1);
    let value = f.call(&source.graph, &f.args.clone());
    f.ret(value);
    f.verify();
    let before = f.graph.clone();
    let sources = HashMap::from_iter([(source.graph.symbol.clone(), source.graph)]);
    let budget = InlineBudget {
        max_depth: 0,
        max_calls: 0,
        max_callee_nodes: 0,
        free_nodes: 0,
        ..InlineBudget::default()
    };
    let stats = inline(&mut f.graph, &sources, budget, 0);
    assert_eq!(stats.calls_inlined, 1);
    assert!(stats.decisions[0].free);
    assert!(calls(&f.graph).is_empty());
    f.verify();
    for value in [Value::Scalar(0), Value::Scalar(u64::MAX), Value::Poison] {
        equivalent(&before, &f.graph, &sources, &[value]);
    }
    let mut limited = before.clone();
    let stats = inline(
        &mut limited,
        &sources,
        InlineBudget {
            max_nodes: 1,
            ..budget
        },
        0,
    );
    assert_eq!(stats.calls_inlined, 0);
    assert_eq!(stats.skipped, [("observations".into(), InlineSkip::Budget)]);
    assert_eq!(limited.dump(), before.dump());
}

#[test]
fn call_site_pruning_precedes_residual_pricing() {
    let (sources, source) = chain(1);
    let mut fixed = caller(&source, Some(32), 1);
    let before = fixed.graph.clone();
    let stats = inline(&mut fixed.graph, &sources, InlineBudget::default(), 0);
    assert!(calls(&fixed.graph).is_empty(), "{stats:?}");
    assert!(stats.specializations_prepared > 0);
    assert!(
        stats
            .decisions
            .iter()
            .any(|decision| decision.specialized_nodes.is_some())
    );
    for value in [Value::Scalar(1), Value::Scalar(u64::MAX), Value::Poison] {
        equivalent(&before, &fixed.graph, &sources, &[value]);
    }
    let mut dynamic = caller(&source, None, 1);
    let before = dynamic.graph.clone();
    let stats = inline(&mut dynamic.graph, &sources, InlineBudget::default(), 0);
    assert_eq!(stats.calls_inlined, 0);
    assert_eq!(stats.skipped, [(source.symbol, InlineSkip::Cost)]);
    for tag in [0, 32, 127, 128, u64::MAX] {
        equivalent(
            &before,
            &dynamic.graph,
            &sources,
            &[Value::Scalar(19), Value::Scalar(tag)],
        );
    }
}

#[test]
fn free_wrappers_do_not_consume_depth_or_costed_call_allowances() {
    let mut leaf = Function::new("leaf", 1);
    let one = leaf.constant(1);
    let result = leaf.node(Opcode::Add, &[leaf.args[0], one], 0, leaf.word);
    leaf.ret(result);
    leaf.verify();
    let mut sources = HashMap::from_iter([("leaf".into(), leaf.graph.clone())]);
    let mut source = leaf.graph;
    for index in 0..8 {
        let mut wrapper = Function::new(&format!("wrapper_{index}"), 1);
        let result = wrapper.call(&source, &wrapper.args.clone());
        wrapper.ret(result);
        wrapper.verify();
        source = wrapper.graph;
        sources.insert(source.symbol.clone(), source.clone());
    }
    let mut f = Function::new("caller", 1);
    let result = f.call(&source, &f.args.clone());
    f.ret(result);
    f.verify();
    let before = f.graph.clone();
    let stats = inline(
        &mut f.graph,
        &sources,
        InlineBudget {
            max_depth: 0,
            max_calls: 0,
            ..InlineBudget::default()
        },
        0,
    );
    assert_eq!(stats.calls_inlined, 9);
    assert!(stats.decisions.iter().all(|decision| decision.free));
    assert!(calls(&f.graph).is_empty());
    f.verify();
    for value in [Value::Scalar(9), Value::Scalar(u64::MAX), Value::Poison] {
        equivalent(&before, &f.graph, &sources, &[value]);
    }
}

#[test]
fn positive_growth_keeps_the_shared_threshold_and_work_limits() {
    let mut source = Function::new("work", 1);
    let mut value = source.args[0];
    for bits in 1..17 {
        let constant = source.constant(bits);
        value = source.node(Opcode::BitXor, &[value, constant], 0, source.word);
    }
    source.ret(value);
    source.verify();
    let cost = estimate_graph(&source.graph, &ResidualCostModel);
    assert!(cost.code_size.total().unwrap() > 3);
    let mut f = Function::new("caller", 1);
    let value = f.call(&source.graph, &f.args.clone());
    f.ret(value);
    f.verify();
    let before = f.graph;
    let sources = HashMap::from_iter([(source.graph.symbol.clone(), source.graph)]);
    for (budget, max_growth, reason) in [
        (InlineBudget::default(), 0, InlineSkip::Cost),
        (
            InlineBudget {
                max_calls: 0,
                ..InlineBudget::default()
            },
            100,
            InlineSkip::Budget,
        ),
        (
            InlineBudget {
                max_depth: 0,
                ..InlineBudget::default()
            },
            100,
            InlineSkip::Budget,
        ),
    ] {
        let mut after = before.clone();
        let stats = inline(&mut after, &sources, budget, max_growth);
        assert_eq!(stats.skipped, [("work".into(), reason)]);
        assert_eq!(after.dump(), before.dump());
        equivalent(&before, &after, &sources, &[Value::Scalar(29)]);
    }
}
