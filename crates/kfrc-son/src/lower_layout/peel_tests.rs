use super::*;
use crate::{ComparePredicate as P, Interpreter, Outcome, verify_graph};
use kfrc_body::{FunctionAbi, IntegerWidth, PassMode, RecordedTypes};

fn projection(g: &mut FunctionGraph, source: NodeId, slot: u32, ty: ValueType) -> NodeId {
    g.add_node(Opcode::Projection, NodeId::NONE, &[source], slot, ty)
}

fn fixture() -> FunctionGraph {
    let mut types = RecordedTypes::new();
    let word = types.intern(
        TypeShape::Int {
            width: IntegerWidth::W8,
            signed: false,
        },
        Some(SizeAlign { size: 1, align: 1 }),
    );
    let boolean = types.intern(TypeShape::Bool, Some(SizeAlign { size: 1, align: 1 }));
    let mut g = FunctionGraph::new(
        "checked_recurrence",
        FunctionAbi {
            args: vec![PassMode::Direct; 2],
            ret: PassMode::Direct,
            c_variadic: false,
        },
        types,
    );
    g.level = GraphLevel::Layout;
    let start = g.add_node(
        Opcode::Start,
        NodeId::NONE,
        &[],
        0,
        ValueType::Tuple(vec![
            ValueType::Control,
            ValueType::Effect,
            ValueType::Rust(word),
            ValueType::Rust(word),
        ]),
    );
    g.start = start;
    let control = projection(&mut g, start, 0, ValueType::Control);
    let effect = projection(&mut g, start, 1, ValueType::Effect);
    let limit = projection(&mut g, start, 2, ValueType::Rust(word));
    let digit = projection(&mut g, start, 3, ValueType::Rust(word));
    let zero = g.add_constant(0, word);
    let one = g.add_constant(1, word);
    let ten = g.add_constant(10, word);
    let entry = g.add_node(Opcode::Jump, control, &[], 0, ValueType::Control);
    let header = g.add_node(
        Opcode::Region,
        NodeId::NONE,
        &[entry],
        0,
        ValueType::Control,
    );
    let i = g.add_node(
        Opcode::Phi,
        NodeId::NONE,
        &[header, zero],
        0,
        ValueType::Rust(word),
    );
    let value = g.add_node(
        Opcode::Phi,
        NodeId::NONE,
        &[header, zero],
        0,
        ValueType::Rust(word),
    );
    let done = g.add_node(
        Opcode::Compare,
        NodeId::NONE,
        &[i, limit],
        P::Equal as u32,
        ValueType::Rust(boolean),
    );
    let test = g.add_node(
        Opcode::If,
        header,
        &[done],
        0,
        ValueType::Tuple(vec![ValueType::Control; 2]),
    );
    let exit = projection(&mut g, test, 0, ValueType::Control);
    let body = projection(&mut g, test, 1, ValueType::Control);
    let product = g.add_node(
        Opcode::Mul,
        NodeId::NONE,
        &[value, ten],
        0,
        ValueType::Rust(word),
    );
    let overflow = g.add_node(
        Opcode::MulOverflow,
        NodeId::NONE,
        &[value, ten],
        0,
        ValueType::Rust(boolean),
    );
    let test = g.add_node(
        Opcode::If,
        body,
        &[overflow],
        0,
        ValueType::Tuple(vec![ValueType::Control; 2]),
    );
    let failed = projection(&mut g, test, 0, ValueType::Control);
    let success = projection(&mut g, test, 1, ValueType::Control);
    let next = g.add_node(
        Opcode::Add,
        NodeId::NONE,
        &[product, digit],
        0,
        ValueType::Rust(word),
    );
    let inc = g.add_node(
        Opcode::Add,
        NodeId::NONE,
        &[i, one],
        0,
        ValueType::Rust(word),
    );
    let latch = g.add_node(Opcode::Jump, success, &[], 0, ValueType::Control);
    g.set_inputs(header, &[entry, latch]);
    g.set_inputs(i, &[header, zero, inc]);
    g.set_inputs(value, &[header, zero, next]);
    // Both distinct exits use loop state, including a floating expression.
    let a = g.add_node(Opcode::Jump, exit, &[], 0, ValueType::Control);
    let b = g.add_node(Opcode::Jump, failed, &[], 0, ValueType::Control);
    let merge = g.add_node(Opcode::Region, NodeId::NONE, &[a, b], 0, ValueType::Control);
    let output = g.add_node(
        Opcode::Phi,
        NodeId::NONE,
        &[merge, value, product],
        0,
        ValueType::Rust(word),
    );
    g.add_node(
        Opcode::Return,
        merge,
        &[effect, output],
        0,
        ValueType::Control,
    );
    verify_graph(&mut g).unwrap();
    g
}

fn execute(g: &FunctionGraph, limit: u64, digit: u64) -> Outcome {
    Interpreter::new(g)
        .run(&[limit, digit], &mut |_: &str, _: &[u64]| unreachable!())
        .unwrap()
}

#[test]
fn checked_recurrence_peels_and_repairs_each_exit() {
    let before = fixture();
    let mut after = before.clone();
    assert_eq!(run(&mut after).loops, 1);
    verify_graph(&mut after).unwrap();
    super::super::cleanup::run(&mut after).unwrap();
    verify_graph(&mut after).unwrap();
    for limit in 0..=16 {
        for digit in 0..=255 {
            assert_eq!(
                execute(&before, limit, digit),
                execute(&after, limit, digit),
                "limit={limit} digit={digit}"
            );
        }
    }
}

#[test]
fn cleanup_removes_only_the_peeled_constant_overflow_guard() {
    let before = fixture();
    let mut after = before.clone();
    assert_eq!(run(&mut after).loops, 1);
    verify_graph(&mut after).unwrap();
    assert_eq!(
        after
            .ids()
            .filter(|&n| after.op(n) == Opcode::MulOverflow)
            .count(),
        2
    );
    super::super::cleanup::run(&mut after).unwrap();
    verify_graph(&mut after).unwrap();
    let overflows: Vec<_> = after
        .ids()
        .filter(|&n| after.op(n) == Opcode::MulOverflow)
        .collect();
    let enabled = std::env::var("KFRC_FOLD_MUL_OVERFLOW").as_deref() != Ok("0");
    assert_eq!(overflows.len(), if enabled { 1 } else { 2 });
    if enabled {
        let recurrence = after.input(overflows[0], 0);
        assert_eq!(after.op(recurrence), Opcode::Phi);
        assert!(!after.ids().any(|n| {
            after.op(n) == Opcode::MulOverflow
                && after
                    .inputs(n)
                    .iter()
                    .all(|&input| after.op(input) == Opcode::Const)
        }));
    }
    for limit in [0, 1, 2, 3, 4, 17, 255] {
        for digit in [0, 1, 2, 9, 25, 127, 255] {
            assert_eq!(
                execute(&before, limit, digit),
                execute(&after, limit, digit)
            );
        }
    }
}

#[test]
fn reverse_header_slots_preserve_state() {
    let mut before = fixture();
    // Find the structural loop rather than depending on fixture NodeIds.
    let cfg = ControlFlow::new(&before);
    let header = cfg.loop_info().loops()[0].header;
    let phis: Vec<_> = before
        .ids()
        .filter(|&n| before.op(n) == Opcode::Phi && before.input(n, 0) == header)
        .collect();
    let edges = before.inputs(header).to_vec();
    before.set_inputs(header, &[edges[1], edges[0]]);
    for phi in phis {
        let v = before.inputs(phi).to_vec();
        before.set_inputs(phi, &[header, v[2], v[1]]);
    }
    verify_graph(&mut before).unwrap();
    let mut after = before.clone();
    assert_eq!(run(&mut after).loops, 1);
    verify_graph(&mut after).unwrap();
    for limit in [0, 1, 2, 3, 4, 17, 255] {
        for digit in [0, 1, 2, 9, 25, 127, 255] {
            assert_eq!(
                execute(&before, limit, digit),
                execute(&after, limit, digit)
            );
        }
    }
}

#[test]
fn ordinary_loops_and_semantic_graphs_stay_unchanged() {
    let mut g = fixture();
    g.level = GraphLevel::Specialized;
    let before = g.dump();
    assert_eq!(run(&mut g).loops, 0);
    assert_eq!(before, g.dump());
    g.level = GraphLevel::Layout;
    let overflows: Vec<_> = g
        .ids()
        .filter(|&n| g.op(n) == Opcode::MulOverflow)
        .collect();
    for node in overflows {
        g.set_opcode(node, Opcode::Compare);
        g.set_payload(node, P::Equal as u32);
    }
    let before = g.dump();
    assert_eq!(run(&mut g).loops, 0);
    assert_eq!(before, g.dump());
}

#[test]
fn effectful_iterations_and_empty_entry_keep_call_order() {
    let mut before = fixture();
    let cfg = ControlFlow::new(&before);
    let loop_ = cfg.loop_info().loops()[0].clone();
    let header = loop_.header;
    let latch = loop_.latches[0];
    let (initial, word) = {
        let phi = before
            .ids()
            .find(|&n| before.op(n) == Opcode::Phi && before.input(n, 0) == header)
            .unwrap();
        (before.input(phi, 1), before.rust_type(phi).unwrap())
    };
    let start = before.start;
    let initial_effect = before
        .ids()
        .find(|&n| {
            before.op(n) == Opcode::Projection
                && before.input(n, 0) == start
                && before.payload(n) == 1
        })
        .unwrap();
    let effect = before.add_node(
        Opcode::Phi,
        NodeId::NONE,
        &[header, initial_effect],
        0,
        ValueType::Effect,
    );
    let symbol = before.add_callee(crate::CallTarget {
        symbol: "iteration".to_owned(),
        abi: FunctionAbi {
            args: vec![PassMode::Direct],
            ret: PassMode::Direct,
            c_variadic: false,
        },
    });
    let call = before.add_node(
        Opcode::Call,
        before.control(latch),
        &[effect, initial],
        symbol,
        ValueType::Tuple(vec![
            ValueType::Control,
            ValueType::Effect,
            ValueType::Rust(word),
        ]),
    );
    let control = projection(&mut before, call, 0, ValueType::Control);
    let next_effect = projection(&mut before, call, 1, ValueType::Effect);
    before.set_control(latch, control);
    before.set_inputs(effect, &[header, initial_effect, next_effect]);
    let ret = before
        .ids()
        .find(|&n| before.op(n) == Opcode::Return)
        .unwrap();
    before.set_input(ret, 0, effect);
    verify_graph(&mut before).unwrap();
    let mut after = before.clone();
    assert_eq!(run(&mut after).loops, 1);
    verify_graph(&mut after).unwrap();
    let execute = |g: &FunctionGraph, limit, digit| {
        let mut calls = Vec::new();
        let result = Interpreter::new(g).run(&[limit, digit], &mut |name: &str, args: &[u64]| {
            calls.push((name.to_owned(), args.to_vec()));
            Ok(0)
        });
        (result, calls)
    };
    for limit in 0..=12 {
        for digit in [0, 1, 2, 9, 25, 128, 255] {
            assert_eq!(
                execute(&before, limit, digit),
                execute(&after, limit, digit)
            );
        }
    }
}
