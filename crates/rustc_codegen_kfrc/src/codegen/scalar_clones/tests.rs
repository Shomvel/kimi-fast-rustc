#[allow(dead_code)]
#[path = "../../../../kfrc-son/tests/semantic_inline_preparation/fixture.rs"]
mod fixture;

use super::*;
use fixture::{Function, equivalent, projection};
use kfrc_body::{SizeAlign, fx_hash::HashMap};
use kfrc_son::{ComparePredicate, Value, ValueType};

fn source() -> FunctionGraph {
    let mut f = Function::new("source", 2);
    let boolean = f
        .graph
        .oracle
        .intern(TypeShape::Bool, Some(SizeAlign { size: 1, align: 1 }));
    for limit in [128, 2048] {
        let bound = f.constant(limit);
        let condition = f.node(
            Opcode::Compare,
            &[f.args[1], bound],
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
        f.control = projection(&mut f.graph, branch, 1, ValueType::Control);
        let result = f.node(Opcode::BitXor, &[f.args[0], bound], 0, f.word);
        f.ret(result);
        f.control = projection(&mut f.graph, branch, 0, ValueType::Control);
    }
    f.ret(f.args[0]);
    f.verify();
    f.graph
}

fn caller(name: &str, target: &FunctionGraph, literal: Option<u64>) -> FunctionGraph {
    let mut f = Function::new(name, 2);
    let arg = literal.map_or(f.args[1], |bits| f.constant(bits));
    let result = f.call(target, &[f.args[0], arg]);
    f.ret(result);
    f.verify();
    f.graph
}

fn job(names: &[(&str, SymbolBinding)]) -> CguJob {
    CguJob {
        pending_functions: names.iter().map(|&(n, b)| (n.into(), b)).collect(),
        needed: BTreeSet::new(),
        data_objects: Vec::new(),
    }
}

#[test]
fn shares_one_clone_without_retargeting_other_literals_or_unknown_arguments() {
    let source = source();
    let mut graphs = CrateGraphs::default();
    for f in [
        caller("a", &source, Some(32)),
        caller("b", &source, Some(32)),
        caller("other", &source, Some(2048)),
        caller("unknown", &source, None),
        source,
    ] {
        graphs.bodies.insert(f.symbol.clone(), f);
    }
    let old = graphs.bodies.clone();
    let visible = SymbolBinding::Global { hidden: false };
    let callers = [
        ("a", visible),
        ("b", visible),
        ("other", visible),
        ("unknown", visible),
    ];

    // A non-hidden declaration wins even if another CGU has a local copy.
    let mut jobs = [
        job(&[("source", SymbolBinding::Local)]),
        job(&[("source", visible)]),
        job(&callers),
    ];
    graphs.specialize_literal_calls(&mut jobs).unwrap();
    assert_eq!(graphs.bodies.len(), old.len());

    jobs[1] = job(&[]);
    graphs.specialize_literal_calls(&mut jobs).unwrap();
    let clone = "source.kfrc.literal.1.20";
    assert_eq!(graphs.bodies.len(), old.len() + 1);
    assert_eq!(jobs[0].pending_functions[clone], SymbolBinding::Local);
    assert_eq!(graphs.bodies[clone].abi, old["source"].abi);
    assert_eq!(branches(&graphs.bodies[clone]), 0);
    assert_eq!(branches(&graphs.bodies["source"]), 2);
    let bodies: HashMap<_, _> = graphs.bodies.into_iter().collect();
    for (name, target) in [
        ("a", clone),
        ("b", clone),
        ("other", "source"),
        ("unknown", "source"),
    ] {
        assert_eq!(fixture::calls(&bodies[name]), [target]);
        for value in [Value::Scalar(5), Value::Poison] {
            for tag in [Value::Scalar(32), Value::Scalar(2048)] {
                equivalent(&old[name], &bodies[name], &bodies, &[value.clone(), tag]);
            }
        }
    }
}
