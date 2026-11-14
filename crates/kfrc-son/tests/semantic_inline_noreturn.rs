#[allow(dead_code)]
#[path = "semantic_inline_preparation/fixture.rs"]
mod fixture;

use fixture::{Function, calls, execute, projection};
use kfrc_body::{PassMode, SizeAlign, TypeShape, fx_hash::HashMap};
use kfrc_son::analysis::function_attrs::infer_exact;
use kfrc_son::{
    CallTarget, ComparePredicate, FunctionGraph, InlineBudget, InterpretError, NodeId, Opcode,
    SemanticOutcome, Value, ValueType, inline_calls, optimize_call_effects, verify_graph,
};
use std::collections::{BTreeMap, BTreeSet};

fn halt() -> FunctionGraph {
    let mut f = Function::new("halt", 1);
    f.call_named("touch", &f.graph.abi.clone(), &f.args.clone());
    f.graph
        .add_node(Opcode::Unreachable, f.control, &[], 0, ValueType::Control);
    f.verify();
    f.graph
}

fn caller(source: &FunctionGraph) -> FunctionGraph {
    let mut f = Function::new("caller", 1);
    let value = f.call(source, &f.args.clone());
    let result = f.call_named("touch", &f.graph.abi.clone(), &[value]);
    f.ret(result);
    f.verify();
    f.graph
}

fn compare(
    before: &FunctionGraph,
    after: &FunctionGraph,
    graphs: &HashMap<String, FunctionGraph>,
    value: Value,
) {
    let original = execute(before, graphs, &[value.clone()]);
    let optimized = execute(after, graphs, &[value]);
    assert_eq!(original.1, optimized.1);
    match (original.0, optimized.0) {
        (Err(InterpretError::UndefinedBehaviour(reason)), Ok(SemanticOutcome::Unreachable)) => {
            assert_eq!(reason, "callee reached unreachable");
        }
        (Err(InterpretError::PoisonObserved(_)), Err(InterpretError::PoisonObserved(_))) => {}
        (original, optimized) => assert_eq!(original, optimized),
    }
}

#[test]
fn divergent_splice_preserves_prior_effects_and_argument_observations() {
    let source = halt();
    let before = caller(&source);
    let graphs = HashMap::from_iter([(source.symbol.clone(), source)]);
    let mut after = before.clone();
    let stats = inline_calls(&mut after, &graphs, InlineBudget::default()).unwrap();
    assert_eq!(stats.calls_inlined, 1);
    assert_eq!(calls(&after), ["touch"]);
    assert!(!after.ids().any(|node| after.op(node) == Opcode::Return));
    for value in [
        Value::Scalar(0),
        Value::Scalar(7),
        Value::Scalar(u64::MAX),
        Value::Poison,
    ] {
        compare(&before, &after, &graphs, value);
    }
}

#[test]
fn divergent_predecessor_prunes_value_and_effect_phi_slots() {
    let source = halt();
    let graphs = HashMap::from_iter([(source.symbol.clone(), source.clone())]);
    for limit in 0..16 {
        let mut f = Function::new("conditional", 1);
        let boolean = f
            .graph
            .oracle
            .intern(TypeShape::Bool, Some(SizeAlign { size: 1, align: 1 }));
        let threshold = f.constant(limit);
        let condition = f.node(
            Opcode::Compare,
            &[f.args[0], threshold],
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
        let hot = projection(&mut f.graph, branch, 0, ValueType::Control);
        let cold = projection(&mut f.graph, branch, 1, ValueType::Control);
        let effect = f.effect;
        f.control = hot;
        let result = f.call(&source, &f.args.clone());
        let hot_effect = f.effect;
        let hot_jump = f
            .graph
            .add_node(Opcode::Jump, f.control, &[], 0, ValueType::Control);
        let cold_jump = f
            .graph
            .add_node(Opcode::Jump, cold, &[], 0, ValueType::Control);
        let region = f.graph.add_node(
            Opcode::Region,
            NodeId::NONE,
            &[hot_jump, cold_jump],
            0,
            ValueType::Control,
        );
        f.control = region;
        f.effect = f.graph.add_node(
            Opcode::Phi,
            NodeId::NONE,
            &[region, hot_effect, effect],
            0,
            ValueType::Effect,
        );
        let value = f.node(Opcode::Phi, &[region, result, f.args[0]], 0, f.word);
        f.ret(value);
        f.verify();
        let before = f.graph.clone();
        inline_calls(&mut f.graph, &graphs, InlineBudget::default()).unwrap();
        assert!(!calls(&f.graph).iter().any(|symbol| symbol == "halt"));
        for value in (0..32).map(Value::Scalar).chain([Value::Poison]) {
            compare(&before, &f.graph, &graphs, value);
        }
    }
}

#[test]
fn a_divergent_call_with_an_ignored_result_needs_no_synthetic_value() {
    let mut source = halt();
    source.abi.ret = PassMode::Ignore;
    verify_graph(&mut source).unwrap();
    let mut f = Function::new("ignored", 1);
    let unit = f
        .graph
        .oracle
        .intern(TypeShape::Unit, Some(SizeAlign { size: 0, align: 1 }));
    let target = f.graph.add_callee(CallTarget {
        symbol: source.symbol.clone(),
        abi: source.abi.clone(),
    });
    let call = f.graph.add_node(
        Opcode::Call,
        f.control,
        &[f.effect, f.args[0]],
        target,
        ValueType::Tuple(vec![
            ValueType::Control,
            ValueType::Effect,
            ValueType::Rust(unit),
        ]),
    );
    f.control = projection(&mut f.graph, call, 0, ValueType::Control);
    f.effect = projection(&mut f.graph, call, 1, ValueType::Effect);
    f.ret(f.args[0]);
    f.verify();
    let before = f.graph.clone();
    let graphs = HashMap::from_iter([(source.symbol.clone(), source)]);
    inline_calls(&mut f.graph, &graphs, InlineBudget::default()).unwrap();
    assert_eq!(calls(&f.graph), ["touch"]);
    compare(&before, &f.graph, &graphs, Value::Scalar(17));
}

#[test]
fn an_infinite_loop_is_a_divergent_source_without_an_unreachable_node() {
    let mut source = Function::new("loops", 1);
    let entry = source
        .graph
        .add_node(Opcode::Jump, source.control, &[], 0, ValueType::Control);
    let region = source.graph.add_node(
        Opcode::Region,
        NodeId::NONE,
        &[entry],
        0,
        ValueType::Control,
    );
    let back = source
        .graph
        .add_node(Opcode::Jump, region, &[], 0, ValueType::Control);
    source.graph.set_inputs(region, &[entry, back]);
    source.verify();
    let before = caller(&source.graph);
    let graphs = HashMap::from_iter([(source.graph.symbol.clone(), source.graph)]);
    let mut after = before.clone();
    inline_calls(&mut after, &graphs, InlineBudget::default()).unwrap();
    assert!(calls(&after).is_empty());
    compare(&before, &after, &graphs, Value::Scalar(7));
}

#[test]
fn noreturn_attributes_cut_continuations_without_erasing_the_call_or_its_effects() {
    let source = halt();
    let before = caller(&source);
    let graphs = HashMap::from_iter([(source.symbol.clone(), source.clone())]);
    let attrs = infer_exact(
        &BTreeMap::from([(source.symbol.clone(), source.clone())]),
        &BTreeSet::from([source.symbol.clone()]),
    );
    assert!(attrs[&source.symbol].noreturn);
    for stale in [false, true] {
        let mut after = before.clone();
        let mut attrs = attrs[&source.symbol].clone();
        if stale {
            attrs.abi.c_variadic = true;
        }
        after.callee_attrs.insert(source.symbol.clone(), attrs);
        let stats = optimize_call_effects(&mut after).unwrap();
        assert_eq!(stats.noreturn_calls, usize::from(!stale));
        assert_eq!(
            calls(&after),
            if stale {
                vec!["halt", "touch"]
            } else {
                vec!["halt"]
            }
        );
        assert_eq!(optimize_call_effects(&mut after).unwrap().noreturn_calls, 0);
        for value in [Value::Scalar(7), Value::Poison] {
            compare(&before, &after, &graphs, value);
        }
    }
}
