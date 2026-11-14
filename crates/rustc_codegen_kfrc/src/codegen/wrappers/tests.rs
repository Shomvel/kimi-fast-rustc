#[allow(dead_code)]
#[path = "../../../../kfrc-son/tests/semantic_inline_preparation/fixture.rs"]
mod fixture;

use super::ScratchGraphs;
use crate::codegen::{CrateGraphs, CrateProvider};
use fixture::{Function, calls, equivalent};
use kfrc_body::fx_hash::HashMap;
use kfrc_son::{GraphProvider, InlineBudget, InlineSkip, Opcode, Value, inline_calls};

fn leaf(name: &str) -> Function {
    let mut f = Function::new(name, 1);
    let one = f.constant(1);
    let value = f.node(Opcode::BitXor, &[f.args[0], one], 0, f.word);
    f.ret(value);
    f.verify();
    f
}

fn wrapper(name: &str, child: &kfrc_son::FunctionGraph) -> Function {
    let mut f = Function::new(name, 1);
    let value = f.call(child, &f.args.clone());
    f.ret(value);
    f.verify();
    f
}

fn simplify(graphs: &mut CrateGraphs) {
    graphs.collect_references();
    if let Err(error) = graphs.simplify_wrappers() {
        panic!("{}: {}", error.symbol, error.message);
    }
}

#[test]
fn private_nested_helpers_become_a_small_reusable_leaf() {
    // Three private edges cannot collapse in the original provider's two
    // immutable waves, but bounded scratch can erase nested dependencies.
    let leaf = leaf("private_leaf").graph;
    let inner = wrapper("inner", &leaf).graph;
    let middle = wrapper("middle", &inner).graph;
    let outer = wrapper("outer", &middle).graph;
    let before = outer.clone();
    let originals = HashMap::from_iter([
        (leaf.symbol.clone(), leaf.clone()),
        (inner.symbol.clone(), inner.clone()),
        (middle.symbol.clone(), middle.clone()),
    ]);
    let mut graphs = CrateGraphs::default();
    graphs.bodies.extend([
        (leaf.symbol.clone(), leaf),
        (inner.symbol.clone(), inner),
        (middle.symbol.clone(), middle),
        (outer.symbol.clone(), outer),
    ]);
    graphs
        .internal
        .extend(["inner".into(), "middle".into(), "private_leaf".into()]);
    graphs.collect_references();
    assert!(CrateProvider { all: &graphs }.graph("middle").is_some());

    simplify(&mut graphs);

    let after = &graphs.bodies["outer"];
    assert!(calls(after).is_empty());
    assert!(after.live_count() <= 64);
    assert!(CrateProvider { all: &graphs }.graph("outer").is_some());
    for argument in [Value::Scalar(0), Value::Scalar(u64::MAX), Value::Poison] {
        equivalent(&before, after, &originals, &[argument]);
    }
}

#[test]
fn publication_exposes_private_dependencies_across_cgus() {
    let private = leaf("private_leaf").graph;
    let nested = wrapper("nested", &private).graph;
    let small = leaf("small").graph;
    let mut rejected = Function::new("rejected", 1);
    let value = rejected.call(&nested, &rejected.args.clone());
    let value = rejected.call(&small, &[value]);
    rejected.ret(value);
    rejected.verify();
    let rejected = rejected.graph;
    let rejected_before = rejected.dump();
    let mut original = Function::new("original", 1);
    let value = original.call(&private, &original.args.clone());
    let value = original.call(&small, &[value]);
    original.ret(value);
    original.verify();
    let before = original.graph.clone();
    let originals = HashMap::from_iter([
        (private.symbol.clone(), private.clone()),
        (small.symbol.clone(), small.clone()),
    ]);
    let mut graphs = CrateGraphs::default();
    for graph in [private, nested, rejected, small, original.graph] {
        graphs.bodies.insert(graph.symbol.clone(), graph);
    }
    graphs
        .internal
        .extend(["private_leaf".into(), "nested".into()]);
    graphs.no_inline.insert("private_leaf".into());
    // Strong intent must not expose a body marked noinline.
    graphs.always_inline.insert("private_leaf".into());

    let mut attempted = graphs.bodies["rejected"].clone();
    inline_calls(
        &mut attempted,
        &ScratchGraphs(&graphs),
        InlineBudget {
            max_depth: 2,
            max_nodes: 256,
            max_calls: 4,
            max_callee_nodes: 64,
            copy_anonymous_constants: true,
            ..InlineBudget::default()
        },
    )
    .unwrap();
    assert_eq!(calls(&attempted), ["private_leaf"]);
    assert!(attempted.live_count() <= 64);
    simplify(&mut graphs);

    assert_ne!(graphs.bodies["rejected"].dump(), rejected_before);
    assert_eq!(calls(&graphs.bodies["rejected"]), ["private_leaf"]);
    assert_eq!(calls(&graphs.bodies["original"]), ["private_leaf"]);
    for argument in [Value::Scalar(7), Value::Poison] {
        equivalent(&before, &graphs.bodies["original"], &originals, &[argument]);
    }
}

#[test]
fn scratch_preserves_inline_intent_without_a_sole_call_size_waiver() {
    let blocked = leaf("blocked").graph;
    let mut large = Function::new("large", 1);
    let mut value = large.args[0];
    for bits in 1..40 {
        let constant = large.constant(bits);
        value = large.node(Opcode::BitXor, &[value, constant], 0, large.word);
    }
    large.ret(value);
    large.verify();
    assert!(large.graph.live_count() > 64);
    let mut caller = wrapper("caller", &large.graph).graph;
    let mut graphs = CrateGraphs::default();
    graphs.bodies.extend([
        (blocked.symbol.clone(), blocked),
        (large.graph.symbol.clone(), large.graph),
        (caller.symbol.clone(), caller.clone()),
    ]);
    graphs.no_inline.insert("blocked".into());
    graphs.always_inline.insert("blocked".into());
    graphs.internal.insert("large".into());
    graphs.collect_references();
    assert!(CrateProvider { all: &graphs }.sole_call_site("large"));
    let scratch = ScratchGraphs(&graphs);
    assert!(!scratch.sole_call_site("large"));
    assert!(scratch.always_inline("blocked"));
    assert!(scratch.graph("blocked").is_none());
    assert!(scratch.preparation_graph("blocked").is_none());
    let stats = inline_calls(
        &mut caller,
        &scratch,
        InlineBudget {
            max_depth: 2,
            max_nodes: 256,
            max_calls: 4,
            max_callee_nodes: 64,
            copy_anonymous_constants: true,
            ..InlineBudget::default()
        },
    )
    .unwrap();
    assert_eq!(stats.skipped, [("large".into(), InlineSkip::Size)]);
    assert_eq!(calls(&caller), ["large"]);

    graphs.always_inline.insert("large".into());
    let mut strong = caller.clone();
    let stats = inline_calls(
        &mut strong,
        &ScratchGraphs(&graphs),
        InlineBudget::default(),
    )
    .unwrap();
    assert_eq!(stats.calls_inlined, 1);
    assert!(calls(&strong).is_empty());
    let stats = inline_calls(
        &mut caller,
        &ScratchGraphs(&graphs),
        InlineBudget {
            max_nodes: 1,
            ..InlineBudget::default()
        },
    )
    .unwrap();
    assert_eq!(stats.skipped, [("large".into(), InlineSkip::Budget)]);
    assert_eq!(calls(&caller), ["large"]);
}
