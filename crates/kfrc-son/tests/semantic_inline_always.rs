//! Strong inline intent changes profitability and work order, while calls keep
//! the same values, observations, effects and shared hard limits.
#[allow(dead_code)]
#[path = "semantic_inline_preparation/fixture.rs"]
mod fixture;

use fixture::{Function, calls, equivalent, guarded};
use kfrc_body::fx_hash::HashMap;
use kfrc_son::analysis::cost::{CostKind, CostModel, GenericCostModel, InstructionCost};
use kfrc_son::{
    FunctionGraph, GraphProvider, InlineBudget, InlineCostPolicy, InlineSkip, NodeId, Opcode,
    Value, inline_calls, inline_calls_with_cost,
};

struct Provider<'a> {
    graphs: &'a HashMap<String, FunctionGraph>,
    always: &'a [&'a str],
    private: Option<&'a str>,
}

impl<'a> Provider<'a> {
    fn new(graphs: &'a HashMap<String, FunctionGraph>, always: &'a [&'a str]) -> Self {
        Self {
            graphs,
            always,
            private: None,
        }
    }
}

impl GraphProvider for Provider<'_> {
    fn graph(&self, symbol: &str) -> Option<&FunctionGraph> {
        self.graphs.get(symbol)
    }

    fn always_inline(&self, symbol: &str) -> bool {
        self.always.contains(&symbol)
    }

    fn can_reference(&self, symbol: &str) -> bool {
        self.private != Some(symbol)
    }
}

fn scalar(name: &str, additions: usize) -> Function {
    let mut f = Function::new(name, 1);
    let one = f.constant(1);
    let mut value = f.args[0];
    for _ in 0..additions {
        value = f.node(Opcode::Add, &[value, one], 0, f.word);
    }
    f.ret(value);
    f.verify();
    f
}

fn caller(sources: &[&FunctionGraph]) -> Function {
    let mut f = Function::new("caller", 1);
    let mut value = f.args[0];
    for source in sources {
        value = f.call(source, &[value]);
    }
    f.ret(value);
    f.verify();
    f
}

struct UnknownCosts;
impl CostModel for UnknownCosts {
    fn name(&self) -> &'static str {
        "test-unknown"
    }

    fn instruction_cost(&self, _: &FunctionGraph, _: NodeId, _: CostKind) -> InstructionCost {
        InstructionCost::Unknown
    }
}

#[test]
fn strong_intent_waives_size_and_cost_but_preserves_values_and_poison() {
    let source = scalar("large", 80);
    assert!(source.graph.live_count() > InlineBudget::default().max_callee_nodes);
    let original = caller(&[&source.graph]).graph;
    let graphs = HashMap::from_iter([("large".into(), source.graph)]);
    let provider = Provider::new(&graphs, &["large"]);
    let cases: [(InlineCostPolicy, &dyn CostModel, InlineSkip); 3] = [
        (
            InlineCostPolicy::Observe,
            &GenericCostModel,
            InlineSkip::Size,
        ),
        (
            InlineCostPolicy::CodeSize {
                max_units: 0,
                free_units: 0,
            },
            &GenericCostModel,
            InlineSkip::Cost,
        ),
        (
            InlineCostPolicy::CodeSize {
                max_units: u64::MAX,
                free_units: u64::MAX,
            },
            &UnknownCosts,
            InlineSkip::UnknownCost,
        ),
    ];
    for (policy, model, rejection) in cases {
        let mut ordinary = original.clone();
        let rejected = inline_calls_with_cost(
            &mut ordinary,
            &graphs,
            InlineBudget::default(),
            policy,
            model,
        )
        .unwrap();
        assert_eq!(rejected.skipped, [("large".into(), rejection)]);
        let mut optimized = original.clone();
        let accepted = inline_calls_with_cost(
            &mut optimized,
            &provider,
            InlineBudget::default(),
            policy,
            model,
        )
        .unwrap();
        assert_eq!(accepted.calls_inlined, 1);
        assert!(calls(&optimized).is_empty());
        for value in [Value::Scalar(0), Value::Scalar(u64::MAX), Value::Poison] {
            equivalent(&original, &optimized, &graphs, &[value]);
        }
    }
}

#[test]
fn strong_intent_keeps_hard_growth_call_and_depth_limits() {
    let source = scalar("large", 80);
    let original = caller(&[&source.graph]).graph;
    let graphs = HashMap::from_iter([("large".into(), source.graph)]);
    let provider = Provider::new(&graphs, &["large"]);
    for budget in [
        InlineBudget {
            max_nodes: 1,
            ..InlineBudget::default()
        },
        InlineBudget {
            max_calls: 0,
            free_nodes: 0,
            ..InlineBudget::default()
        },
        InlineBudget {
            max_depth: 0,
            ..InlineBudget::default()
        },
    ] {
        let mut graph = original.clone();
        let stats = inline_calls(&mut graph, &provider, budget).unwrap();
        assert_eq!(stats.calls_inlined, 0);
        assert_eq!(stats.skipped, [("large".into(), InlineSkip::Budget)]);
        assert_eq!(graph.dump(), original.dump());
    }
    let mut repeated = caller(&[&graphs["large"], &graphs["large"]]);
    let before = repeated.graph.clone();
    let budget = InlineBudget {
        max_nodes: graphs["large"].live_count() + 16,
        free_nodes: 0,
        ..InlineBudget::default()
    };
    let stats = inline_calls(&mut repeated.graph, &provider, budget).unwrap();
    assert_eq!(stats.calls_inlined, 1);
    assert!(stats.nodes_added <= budget.max_nodes);
    assert_eq!(calls(&repeated.graph), ["large"]);
    equivalent(&before, &repeated.graph, &graphs, &[Value::Scalar(7)]);
}

#[test]
fn strong_intent_precedes_smaller_calls_without_resetting_the_call_account() {
    let ordinary = scalar("ordinary", 32);
    let strong = scalar("strong", 80);
    let mut f = caller(&[&ordinary.graph, &strong.graph]);
    let before = f.graph.clone();
    let graphs = HashMap::from_iter([
        ("ordinary".into(), ordinary.graph),
        ("strong".into(), strong.graph),
    ]);
    let stats = inline_calls(
        &mut f.graph,
        &Provider::new(&graphs, &["strong"]),
        InlineBudget {
            max_calls: 1,
            free_nodes: 0,
            ..InlineBudget::default()
        },
    )
    .unwrap();
    assert_eq!(stats.calls_inlined, 1);
    assert_eq!(stats.skipped, [("ordinary".into(), InlineSkip::Budget)]);
    assert_eq!(calls(&f.graph), ["ordinary"]);
    equivalent(&before, &f.graph, &graphs, &[Value::Scalar(7)]);
}

#[test]
fn newly_exposed_strong_calls_join_the_same_priority_queue() {
    let ordinary = scalar("ordinary", 32);
    let strong = scalar("strong", 80);
    let mut wrapper = Function::new("wrapper", 1);
    let result = wrapper.call(&strong.graph, &wrapper.args.clone());
    wrapper.ret(result);
    wrapper.verify();
    let mut f = caller(&[&wrapper.graph, &ordinary.graph]);
    let before = f.graph.clone();
    let graphs = HashMap::from_iter([
        ("ordinary".into(), ordinary.graph),
        ("strong".into(), strong.graph),
        ("wrapper".into(), wrapper.graph),
    ]);
    let stats = inline_calls(
        &mut f.graph,
        &Provider::new(&graphs, &["strong"]),
        InlineBudget {
            max_calls: 2,
            free_nodes: 0,
            ..InlineBudget::default()
        },
    )
    .unwrap();
    assert_eq!(stats.calls_inlined, 2);
    assert_eq!(calls(&f.graph), ["ordinary"]);
    equivalent(&before, &f.graph, &graphs, &[Value::Scalar(7)]);
}

#[test]
fn preparation_forwards_strong_intent_without_relying_on_emission_depth() {
    let child = scalar("child", 80);
    let source = guarded("outer", &child.graph);
    let mut f = fixture::caller(&source.graph, Some(32), 1);
    let before = f.graph.clone();
    let graphs = HashMap::from_iter([
        ("child".into(), child.graph),
        ("outer".into(), source.graph),
    ]);
    let stats = inline_calls(
        &mut f.graph,
        &Provider::new(&graphs, &["outer", "child"]),
        InlineBudget {
            // A child left for emission would be rejected at depth one.
            max_depth: 1,
            ..InlineBudget::default()
        },
    )
    .unwrap();
    assert!(stats.preparation.decisions.iter().any(|decision| {
        decision.symbol == "child" && decision.action == "splice" && decision.rejection.is_none()
    }));
    assert!(calls(&f.graph).is_empty());
    assert!(stats.preparation.clone_budget_used <= InlineBudget::default().max_nodes);
    equivalent(&before, &f.graph, &graphs, &[Value::Scalar(7)]);
}

#[test]
fn strong_intent_preserves_recursion_abi_and_private_reference_rejections() {
    for expected in [
        InlineSkip::Recursive,
        InlineSkip::AbiMismatch,
        InlineSkip::PrivateReference,
    ] {
        let mut source = Function::new("guarded", 1);
        let target = if expected == InlineSkip::Recursive {
            "guarded"
        } else {
            "foreign_private"
        };
        let abi = source.graph.abi.clone();
        let result = source.call_named(target, &abi, &source.args.clone());
        source.ret(result);
        source.verify();
        let mut f = caller(&[&source.graph]);
        let before = f.graph.dump();
        if expected == InlineSkip::AbiMismatch {
            source.graph.abi.c_variadic = true;
        }
        let graphs = HashMap::from_iter([("guarded".into(), source.graph)]);
        let mut provider = Provider::new(&graphs, &["guarded"]);
        provider.private = Some("foreign_private");
        let stats = inline_calls(&mut f.graph, &provider, InlineBudget::default()).unwrap();
        assert_eq!(stats.calls_inlined, 0);
        assert_eq!(stats.skipped, [("guarded".into(), expected)]);
        assert_eq!(f.graph.dump(), before);
    }
}

#[test]
fn strong_intent_does_not_expand_local_data_copy_permission() {
    // Identity and writable data stay protected even with the existing explicit
    // copy option. Anonymous readonly data still needs a loop hint.
    for (unnamed, writable, copy) in [
        (false, false, true),
        (true, true, true),
        (true, false, false),
    ] {
        let mut source = Function::new("data_user", 1);
        let address = source.address("local_data", writable);
        source.graph.data_objects[0].unnamed_address = unnamed;
        source.observe(address);
        source.ret(source.args[0]);
        source.verify();
        let mut f = caller(&[&source.graph]);
        let before = f.graph.dump();
        let graphs = HashMap::from_iter([("data_user".into(), source.graph)]);
        let stats = inline_calls(
            &mut f.graph,
            &Provider::new(&graphs, &["data_user"]),
            InlineBudget {
                copy_anonymous_constants: copy,
                ..InlineBudget::default()
            },
        )
        .unwrap();
        assert_eq!(stats.calls_inlined, 0);
        assert_eq!(
            stats.skipped,
            [("data_user".into(), InlineSkip::DataConflict)]
        );
        assert_eq!(f.graph.dump(), before);
        assert!(f.graph.data_objects.is_empty());
    }
}
