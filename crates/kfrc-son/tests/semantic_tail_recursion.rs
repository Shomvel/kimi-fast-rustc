#[allow(dead_code)]
#[path = "semantic_inline_preparation/fixture.rs"]
mod fixture;

use fixture::{Function, equivalent, projection};
use kfrc_body::fx_hash::HashMap;
use kfrc_body::{Mutability, SizeAlign, TypeShape};
use kfrc_son::{
    ComparePredicate, Opcode, Value, ValueType, eliminate_tail_recursion, verify_graph,
};

fn sum(poison: bool, swap: bool) -> Function {
    let mut f = Function::new("tail_sum", 2);
    let boolean = f
        .graph
        .oracle
        .intern(TypeShape::Bool, Some(SizeAlign { size: 1, align: 1 }));
    let zero = f.constant(0);
    let done = f.node(
        Opcode::Compare,
        &[f.args[0], zero],
        ComparePredicate::Equal as u32,
        boolean,
    );
    let branch = f.graph.add_node(
        Opcode::If,
        f.control,
        &[done],
        0,
        ValueType::Tuple(vec![ValueType::Control; 2]),
    );
    let effect = f.effect;
    f.control = projection(&mut f.graph, branch, 0, ValueType::Control);
    f.ret(f.args[1]);
    f.control = projection(&mut f.graph, branch, 1, ValueType::Control);
    f.effect = effect;
    let one = f.constant(1);
    let n = f.node(Opcode::Sub, &[f.args[0], one], 0, f.word);
    let acc = if poison {
        f.node(Opcode::Undefined, &[], 0, f.word)
    } else if swap {
        f.args[0]
    } else {
        f.node(Opcode::Add, &[f.args[0], f.args[1]], 0, f.word)
    };
    let abi = f.graph.abi.clone();
    let value = f.call_named("tail_sum", &abi, &[n, acc]);
    f.ret(value);
    f.verify();
    f
}

#[test]
fn recursion_becomes_a_loop_with_simultaneous_argument_updates() {
    for swap in [false, true] {
        let before = sum(false, swap).graph;
        let mut after = before.clone();
        let stats = eliminate_tail_recursion(&mut after).unwrap();
        assert_eq!(stats.calls_eliminated, 1);
        verify_graph(&mut after).unwrap();
        assert!(!after.ids().any(|n| after.op(n) == Opcode::Call));
        let graphs = HashMap::from_iter([(before.symbol.clone(), before.clone())]);
        for n in 0..24 {
            equivalent(
                &before,
                &after,
                &graphs,
                &[Value::Scalar(n), Value::Scalar(7)],
            );
        }
    }
}

#[test]
fn erased_calls_still_observe_poison_in_every_argument() {
    let before = sum(true, false).graph;
    let mut after = before.clone();
    assert_eq!(
        eliminate_tail_recursion(&mut after)
            .unwrap()
            .calls_eliminated,
        1
    );
    let graphs = HashMap::from_iter([(before.symbol.clone(), before.clone())]);
    for n in [0, 1, 5] {
        equivalent(
            &before,
            &after,
            &graphs,
            &[Value::Scalar(n), Value::Scalar(3)],
        );
    }
}

#[test]
fn local_frame_storage_disables_loop_conversion() {
    let mut f = sum(false, false);
    let pointer = f.graph.oracle.intern(
        TypeShape::RawPointer {
            mutability: Mutability::Mut,
            pointee: f.word,
        },
        Some(SizeAlign { size: 8, align: 8 }),
    );
    f.node(Opcode::LocalSlot, &[], 0, pointer);
    assert_eq!(
        eliminate_tail_recursion(&mut f.graph)
            .unwrap()
            .calls_eliminated,
        0
    );
}

#[test]
fn work_after_call_and_abi_mismatch_stay_calls() {
    let mut f = sum(false, false);
    let ret = f
        .graph
        .ids()
        .find(|&n| {
            f.graph.op(n) == Opcode::Return
                && f.graph.op(f.graph.input(n, 1)) == Opcode::Projection
                && f.graph.input(f.graph.input(n, 1), 0) != f.graph.start
        })
        .unwrap();
    let one = f.constant(1);
    let value = f.node(Opcode::Add, &[f.graph.input(ret, 1), one], 0, f.word);
    f.graph.set_input(ret, 1, value);
    assert_eq!(
        eliminate_tail_recursion(&mut f.graph)
            .unwrap()
            .calls_eliminated,
        0
    );
    let mut f = sum(false, false);
    f.graph.abi.c_variadic = true;
    assert_eq!(
        eliminate_tail_recursion(&mut f.graph)
            .unwrap()
            .calls_eliminated,
        0
    );
}

#[test]
fn effects_before_recursion_remain_in_each_iteration() {
    let mut f = sum(false, false);
    let call = f
        .graph
        .ids()
        .find(|&n| f.graph.op(n) == Opcode::Call)
        .unwrap();
    let original = f.graph.control(call);
    let effect = f.graph.input(call, 0);
    let abi = f.graph.abi.clone();
    f.control = original;
    f.effect = effect;
    let args = f.args.clone();
    f.call_named("touch", &abi, &args);
    f.graph.set_control(call, f.control);
    f.graph.set_input(call, 0, f.effect);
    f.verify();
    let before = f.graph.clone();
    assert_eq!(
        eliminate_tail_recursion(&mut f.graph)
            .unwrap()
            .calls_eliminated,
        1
    );
    let graphs = HashMap::from_iter([(before.symbol.clone(), before.clone())]);
    equivalent(
        &before,
        &f.graph,
        &graphs,
        &[Value::Scalar(9), Value::Scalar(7)],
    );
}

#[test]
fn shared_return_phis_are_split_before_loop_formation() {
    let mut f = sum(false, false);
    let returns: Vec<_> = f
        .graph
        .ids()
        .filter(|&n| f.graph.op(n) == Opcode::Return)
        .collect();
    let mut jumps = Vec::new();
    let mut values = Vec::new();
    let mut effects = Vec::new();
    for ret in returns {
        values.push(f.graph.input(ret, 1));
        effects.push(f.graph.input(ret, 0));
        jumps.push(f.graph.add_node(
            Opcode::Jump,
            f.graph.control(ret),
            &[],
            0,
            ValueType::Control,
        ));
        f.graph.remove_node(ret);
    }
    let region = f.graph.add_node(
        Opcode::Region,
        kfrc_son::NodeId::NONE,
        &jumps,
        0,
        ValueType::Control,
    );
    values.insert(0, region);
    effects.insert(0, region);
    let value = f.graph.add_node(
        Opcode::Phi,
        kfrc_son::NodeId::NONE,
        &values,
        0,
        ValueType::Rust(f.word),
    );
    f.effect = f.graph.add_node(
        Opcode::Phi,
        kfrc_son::NodeId::NONE,
        &effects,
        0,
        ValueType::Effect,
    );
    f.control = region;
    f.ret(value);
    f.verify();
    let before = f.graph.clone();
    let stats = eliminate_tail_recursion(&mut f.graph).unwrap();
    assert_eq!(stats.returns_duplicated, 1);
    assert_eq!(stats.calls_eliminated, 1);
    verify_graph(&mut f.graph).unwrap();
    let graphs = HashMap::from_iter([(before.symbol.clone(), before.clone())]);
    for n in 0..16 {
        equivalent(
            &before,
            &f.graph,
            &graphs,
            &[Value::Scalar(n), Value::Scalar(7)],
        );
    }
}
