//! Call-site profitability changes budgets without changing observable work.
#[allow(dead_code)]
#[path = "semantic_inline_preparation/fixture.rs"]
mod fixture;
use fixture::{Function, equivalent, projection};
use kfrc_body::fx_hash::HashMap;
use kfrc_body::{SizeAlign, TypeShape};
use kfrc_son::analysis::cost::ResidualCostModel;
use kfrc_son::{
    ComparePredicate, FunctionGraph, InlineBudget, InlineCostPolicy, InlineSkip, InlineTemperature,
    NodeId, Opcode, Value, ValueType, inline_calls_with_cost,
};

const POLICY: InlineCostPolicy = InlineCostPolicy::CallSiteResidual {
    max_growth: 32,
    hot_growth: 256,
    total_hot_growth: 512,
};

fn work() -> FunctionGraph {
    let mut f = Function::new("work", 2);
    let mut value = f.args[0];
    for _ in 0..24 {
        value = f.node(Opcode::BitXor, &[value, f.args[1]], 0, f.word);
        value = f.node(Opcode::Mul, &[value, f.args[1]], 0, f.word);
    }
    f.ret(value);
    f.verify();
    f.graph
}

fn loop_caller(target: &FunctionGraph) -> Function {
    loop_caller_with_targets(&[target])
}

fn loop_caller_with_targets(targets: &[&FunctionGraph]) -> Function {
    let mut f = Function::new("loop_caller", 3);
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
        &[iv, f.args[2]],
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
    for &target in targets {
        let mut args = vec![next_result, f.args[1]];
        if target.abi.args.len() == 3 {
            args.push(f.constant(7));
        }
        next_result = f.call(target, &args);
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

fn optimize(
    f: &mut Function,
    sources: &HashMap<String, FunctionGraph>,
    policy: InlineCostPolicy,
    budget: InlineBudget,
) -> kfrc_son::InlineStats {
    inline_calls_with_cost(&mut f.graph, sources, budget, policy, &ResidualCostModel).unwrap()
}

fn equivalent_loop(
    before: &FunctionGraph,
    after: &FunctionGraph,
    sources: &HashMap<String, FunctionGraph>,
) {
    for seed in [0, 1, u64::MAX, 0x8123_4567_89ab_cdef] {
        for key in [1, 3, 37, u64::MAX] {
            for count in [0, 1, 2, 7] {
                equivalent(
                    before,
                    after,
                    sources,
                    &[
                        Value::Scalar(seed),
                        Value::Scalar(key),
                        Value::Scalar(count),
                    ],
                );
            }
        }
    }
    equivalent(
        before,
        after,
        sources,
        &[Value::Poison, Value::Poison, Value::Scalar(0)],
    );
}

#[test]
fn cyclic_sites_get_growth_while_ordinary_sites_and_uniform_controls_keep_their_limits() {
    let source = work();
    let sources = HashMap::from_iter([(source.symbol.clone(), source.clone())]);
    for (cyclic, policy, inlined, temperature) in [
        (true, POLICY, 1, InlineTemperature::Hot),
        (false, POLICY, 0, InlineTemperature::Ordinary),
        (
            true,
            InlineCostPolicy::Residual { max_growth: 32 },
            0,
            InlineTemperature::Unknown,
        ),
    ] {
        let mut f = if cyclic {
            loop_caller(&source)
        } else {
            let mut f = Function::new("ordinary", 3);
            let result = f.call(&source, &f.args[..2].to_vec());
            f.ret(result);
            f.verify();
            f
        };
        let before = f.graph.clone();
        let stats = optimize(&mut f, &sources, policy, InlineBudget::default());
        assert_eq!(stats.calls_inlined, inlined, "{stats:?}");
        assert_eq!(stats.decisions[0].temperature, temperature);
        assert_eq!(
            stats.decisions[0].rejection,
            (inlined == 0).then_some(InlineSkip::Cost)
        );
        f.verify();
        equivalent_loop(&before, &f.graph, &sources);
    }
}

/// The third argument triggers cached preparation without determining the
/// branch. Only the returning arm inherits the loop caller's hot allowance.
fn guarded(source: &FunctionGraph) -> FunctionGraph {
    let mut f = Function::new("guarded", 3);
    let boolean = f
        .graph
        .oracle
        .intern(TypeShape::Bool, Some(SizeAlign { size: 1, align: 1 }));
    let zero = f.constant(0);
    let condition = f.node(
        Opcode::Compare,
        &[f.args[1], zero],
        ComparePredicate::Equal as u32,
        boolean,
    );
    let branch = f.graph.add_node(
        Opcode::If,
        f.control,
        &[condition],
        0,
        ValueType::Tuple(vec![ValueType::Control; 2]),
    );
    let effect = f.effect;
    f.control = projection(&mut f.graph, branch, 0, ValueType::Control);
    f.call(source, &f.args[..2].to_vec());
    f.graph
        .add_node(Opcode::Unreachable, f.control, &[], 0, ValueType::Control);
    f.control = projection(&mut f.graph, branch, 1, ValueType::Control);
    f.effect = effect;
    let result = f.call(source, &f.args[..2].to_vec());
    let result = f.node(Opcode::Add, &[result, f.args[2]], 0, f.word);
    f.ret(result);
    f.verify();
    f.graph
}

#[test]
fn prepared_sources_keep_cold_paths_cold_when_spliced_into_hot_loops() {
    let source = work();
    let guarded = guarded(&source);
    let mut f = loop_caller(&guarded);
    let before = f.graph.clone();
    let sources = HashMap::from_iter([
        (source.symbol.clone(), source),
        (guarded.symbol.clone(), guarded),
    ]);
    let stats = optimize(&mut f, &sources, POLICY, InlineBudget::default());
    assert_eq!(stats.calls_inlined, 2, "{stats:?}");
    assert!(stats.specializations_prepared > 0);
    assert!(
        stats
            .preparation
            .decisions
            .iter()
            .any(|decision| decision.symbol == "work" && decision.action == "splice")
    );
    assert!(
        stats
            .decisions
            .iter()
            .any(|decision| decision.symbol == "work"
                && decision.temperature == InlineTemperature::Hot
                && decision.rejection.is_none())
    );
    assert!(
        stats
            .decisions
            .iter()
            .any(|decision| decision.symbol == "work"
                && decision.temperature == InlineTemperature::Cold
                && decision.rejection == Some(InlineSkip::Cost))
    );
    assert!(
        stats
            .preparation
            .decisions
            .iter()
            .filter(|decision| decision.symbol == "work" && decision.action == "splice")
            .all(|decision| decision.rejection == Some(InlineSkip::Cost))
    );
    f.verify();
    equivalent_loop(&before, &f.graph, &sources);
}

#[test]
fn hot_hints_do_not_waive_growth_depth_or_costed_call_budgets() {
    let source = work();
    let sources = HashMap::from_iter([(source.symbol.clone(), source.clone())]);
    for budget in [
        InlineBudget {
            max_nodes: 1,
            ..InlineBudget::default()
        },
        InlineBudget {
            max_calls: 0,
            ..InlineBudget::default()
        },
        InlineBudget {
            max_depth: 0,
            ..InlineBudget::default()
        },
    ] {
        let mut f = loop_caller(&source);
        let before = f.graph.clone();
        let stats = optimize(&mut f, &sources, POLICY, budget);
        assert_eq!(stats.calls_inlined, 0);
        assert_eq!(stats.hot_growth_spent, 0);
        assert_eq!(stats.skipped, [("work".into(), InlineSkip::Budget)]);
        assert_eq!(before.dump(), f.graph.dump());
        equivalent_loop(&before, &f.graph, &sources);
    }
}

#[test]
fn classification_exhaustion_uses_the_ordinary_threshold() {
    let source = work();
    let sources = HashMap::from_iter([(source.symbol.clone(), source.clone())]);
    let mut f = loop_caller(&source);
    for bits in 0..26_000 {
        f.constant(bits);
    }
    f.verify();
    let before = f.graph.clone();
    let stats = optimize(&mut f, &sources, POLICY, InlineBudget::default());
    assert_eq!(stats.calls_inlined, 0);
    assert_eq!(stats.skipped, [("work".into(), InlineSkip::Cost)]);
    assert_eq!(stats.temperature_work, 100_000);
    assert_eq!(stats.temperature_budget_bailouts, 1);
    assert_eq!(stats.decisions[0].temperature, InlineTemperature::Unknown);
    assert_eq!(before.dump(), f.graph.dump());
    equivalent_loop(&before, &f.graph, &sources);
}

#[test]
fn successful_hot_sites_share_one_extra_growth_account() {
    let source = work();
    let sources = HashMap::from_iter([(source.symbol.clone(), source.clone())]);
    let mut f = loop_caller_with_targets(&[&source; 4]);
    let before = f.graph.clone();
    let stats = optimize(
        &mut f,
        &sources,
        InlineCostPolicy::CallSiteResidual {
            max_growth: 32,
            hot_growth: 256,
            total_hot_growth: 24,
        },
        InlineBudget::default(),
    );
    // The 48 operations replace a four-unit call: each successful splice
    // spends 48 - 4 - 32 = 12 extra units. Rejections spend nothing.
    assert_eq!(stats.calls_inlined, 2, "{stats:?}");
    assert_eq!(stats.hot_growth_spent, 24);
    assert_eq!(stats.decisions.len(), 4);
    for (index, decision) in stats.decisions.iter().enumerate() {
        assert_eq!(decision.estimate.unwrap().code_size.total(), Some(48));
        assert_eq!(decision.call_units, Some(4));
        assert_eq!(decision.hot_growth_remaining, Some([24, 12, 0, 0][index]));
        assert_eq!(decision.hot_growth_charged, [12, 12, 0, 0][index]);
        assert_eq!(decision.rejection, (index >= 2).then_some(InlineSkip::Cost));
    }
    f.verify();
    equivalent_loop(&before, &f.graph, &sources);
}

#[test]
fn nested_hot_sites_cannot_reset_the_callers_extra_growth_account() {
    let source = work();
    let mut wrapper = Function::new("wrapper", 2);
    let first = wrapper.call(&source, &wrapper.args.clone());
    let second = wrapper.call(&source, &[first, wrapper.args[1]]);
    wrapper.ret(second);
    wrapper.verify();
    let mut f = loop_caller(&wrapper.graph);
    let before = f.graph.clone();
    let sources = HashMap::from_iter([
        (source.symbol.clone(), source),
        (wrapper.graph.symbol.clone(), wrapper.graph),
    ]);
    let stats = optimize(
        &mut f,
        &sources,
        InlineCostPolicy::CallSiteResidual {
            max_growth: 32,
            hot_growth: 256,
            total_hot_growth: 12,
        },
        InlineBudget::default(),
    );
    assert_eq!(stats.calls_inlined, 2, "{stats:?}");
    assert_eq!(stats.hot_growth_spent, 12);
    let children: Vec<_> = stats
        .decisions
        .iter()
        .filter(|decision| decision.symbol == "work")
        .collect();
    assert_eq!(children.len(), 2);
    assert!(children.iter().all(|decision| decision.depth == 1));
    assert_eq!(children[0].hot_growth_charged, 12);
    assert_eq!(children[1].hot_growth_remaining, Some(0));
    assert_eq!(children[1].rejection, Some(InlineSkip::Cost));
    f.verify();
    equivalent_loop(&before, &f.graph, &sources);
}

#[test]
fn no_hot_credit_still_allows_ordinary_growth() {
    let large = work();
    let mut small = Function::new("ordinary_growth", 2);
    let mut value = small.args[0];
    for _ in 0..12 {
        value = small.node(Opcode::BitXor, &[value, small.args[1]], 0, small.word);
        value = small.node(Opcode::Mul, &[value, small.args[1]], 0, small.word);
    }
    small.ret(value);
    small.verify();
    let mut f = loop_caller_with_targets(&[&large, &small.graph]);
    let before = f.graph.clone();
    let sources = HashMap::from_iter([
        (large.symbol.clone(), large),
        (small.graph.symbol.clone(), small.graph),
    ]);
    let stats = optimize(
        &mut f,
        &sources,
        InlineCostPolicy::CallSiteResidual {
            max_growth: 32,
            hot_growth: 256,
            total_hot_growth: 0,
        },
        InlineBudget::default(),
    );
    assert_eq!(stats.calls_inlined, 1, "{stats:?}");
    assert_eq!(stats.hot_growth_spent, 0);
    let ordinary = stats
        .decisions
        .iter()
        .find(|decision| decision.symbol == "ordinary_growth")
        .unwrap();
    assert_eq!(ordinary.rejection, None);
    assert!(!ordinary.free);
    assert_eq!(ordinary.hot_growth_remaining, Some(0));
    assert_eq!(ordinary.hot_growth_charged, 0);
    f.verify();
    equivalent_loop(&before, &f.graph, &sources);
}

#[test]
fn sole_and_strong_inline_exceptions_do_not_spend_hot_credit() {
    struct Intent<'a> {
        sources: &'a HashMap<String, FunctionGraph>,
        strong: bool,
    }
    impl kfrc_son::GraphProvider for Intent<'_> {
        fn graph(&self, symbol: &str) -> Option<&FunctionGraph> {
            self.sources.get(symbol)
        }
        fn always_inline(&self, symbol: &str) -> bool {
            self.strong && symbol == "work"
        }
        fn sole_call_site(&self, symbol: &str) -> bool {
            !self.strong && symbol == "work"
        }
    }
    let source = work();
    let sources = HashMap::from_iter([(source.symbol.clone(), source.clone())]);
    for strong in [false, true] {
        let mut f = loop_caller(&source);
        let before = f.graph.clone();
        let stats = inline_calls_with_cost(
            &mut f.graph,
            &Intent {
                sources: &sources,
                strong,
            },
            InlineBudget::default(),
            InlineCostPolicy::CallSiteResidual {
                max_growth: 32,
                hot_growth: 256,
                total_hot_growth: 0,
            },
            &ResidualCostModel,
        )
        .unwrap();
        assert_eq!(stats.calls_inlined, 1, "{stats:?}");
        assert_eq!(stats.hot_growth_spent, 0);
        assert_eq!(stats.decisions[0].hot_growth_charged, 0);
        f.verify();
        equivalent_loop(&before, &f.graph, &sources);
    }
}
