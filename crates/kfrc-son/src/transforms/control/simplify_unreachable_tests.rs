//! Branches into Unreachable and the Region entries that commit to them.
use super::*;
use crate::{CallTarget, ComparePredicate, GraphLevel, InterpretError, Interpreter, verify_graph};
use kfrc_body::{FunctionAbi, IntegerWidth, PassMode, RecordedTypes, SizeAlign};

fn projection(graph: &mut FunctionGraph, source: NodeId, slot: u32, ty: ValueType) -> NodeId {
    graph.add_node(Opcode::Projection, NodeId::NONE, &[source], slot, ty)
}

fn branch(graph: &mut FunctionGraph, control: NodeId, condition: NodeId) -> (NodeId, NodeId) {
    let node = graph.add_node(
        Opcode::If,
        control,
        &[condition],
        0,
        ValueType::Tuple(vec![ValueType::Control; 2]),
    );
    (
        projection(graph, node, 0, ValueType::Control),
        projection(graph, node, 1, ValueType::Control),
    )
}

/// `let p = if x == 0 { [observe(x);] 0 } else { x }; if p == 0 { unreachable } p`
fn fixture(observed: bool) -> FunctionGraph {
    let mut types = RecordedTypes::new();
    let word = types.intern(
        TypeShape::Int {
            width: IntegerWidth::W64,
            signed: false,
        },
        Some(SizeAlign { size: 8, align: 8 }),
    );
    let boolean = types.intern(TypeShape::Bool, Some(SizeAlign { size: 1, align: 1 }));
    let mut graph = FunctionGraph::new(
        "unreachable_entry",
        FunctionAbi {
            args: vec![PassMode::Direct],
            ret: PassMode::Direct,
            c_variadic: false,
        },
        types,
    );
    graph.level = GraphLevel::Layout;
    let start = graph.add_node(
        Opcode::Start,
        NodeId::NONE,
        &[],
        0,
        ValueType::Tuple(vec![
            ValueType::Control,
            ValueType::Effect,
            ValueType::Rust(word),
        ]),
    );
    graph.start = start;
    let control = projection(&mut graph, start, 0, ValueType::Control);
    let effect = projection(&mut graph, start, 1, ValueType::Effect);
    let x = projection(&mut graph, start, 2, ValueType::Rust(word));
    let zero = graph.add_constant(0, word);
    let is_zero = graph.add_node(
        Opcode::Compare,
        NodeId::NONE,
        &[x, zero],
        ComparePredicate::Equal as u32,
        ValueType::Rust(boolean),
    );
    let (mut zero_path, other_path) = branch(&mut graph, control, is_zero);
    let mut zero_effect = effect;
    if observed {
        let target = graph.add_callee(CallTarget {
            symbol: "observe".into(),
            abi: FunctionAbi {
                args: vec![PassMode::Direct],
                ret: PassMode::Direct,
                c_variadic: false,
            },
        });
        let call = graph.add_node(
            Opcode::Call,
            zero_path,
            &[effect, x],
            target,
            ValueType::Tuple(vec![
                ValueType::Control,
                ValueType::Effect,
                ValueType::Rust(word),
            ]),
        );
        zero_path = projection(&mut graph, call, 0, ValueType::Control);
        zero_effect = projection(&mut graph, call, 1, ValueType::Effect);
    }
    let jumps = [zero_path, other_path]
        .map(|control| graph.add_node(Opcode::Jump, control, &[], 0, ValueType::Control));
    let region = graph.add_node(Opcode::Region, NodeId::NONE, &jumps, 0, ValueType::Control);
    let effect = graph.add_node(
        Opcode::Phi,
        NodeId::NONE,
        &[region, zero_effect, effect],
        0,
        ValueType::Effect,
    );
    let merged = graph.add_node(
        Opcode::Phi,
        NodeId::NONE,
        &[region, zero, x],
        0,
        ValueType::Rust(word),
    );
    let undefined = graph.add_node(
        Opcode::Compare,
        NodeId::NONE,
        &[merged, zero],
        ComparePredicate::Equal as u32,
        ValueType::Rust(boolean),
    );
    let (dead, live) = branch(&mut graph, region, undefined);
    graph.add_node(Opcode::Unreachable, dead, &[], 0, ValueType::Control);
    graph.add_node(
        Opcode::Return,
        live,
        &[effect, merged],
        0,
        ValueType::Control,
    );
    graph
}

fn execute(graph: &FunctionGraph, input: u64) -> (String, Vec<String>) {
    let mut calls = Vec::new();
    let outcome = Interpreter::new(graph).run(&[input], &mut |name: &str, _: &[u64]| {
        calls.push(name.to_owned());
        Ok::<_, InterpretError>(7)
    });
    (format!("{outcome:?}"), calls)
}

fn count(graph: &FunctionGraph, op: Opcode) -> usize {
    graph.ids().filter(|&node| graph.op(node) == op).count()
}

#[test]
fn entries_committing_to_unreachable_are_cut_and_both_branches_fold() {
    let mut graph = fixture(false);
    verify_graph(&mut graph).unwrap();
    let original = graph.clone();
    run_final(&mut graph).unwrap();
    verify_graph(&mut graph).unwrap_or_else(|error| panic!("{error}\n{}", graph.dump()));
    assert_eq!(count(&graph, Opcode::If), 0, "{}", graph.dump());
    assert_eq!(count(&graph, Opcode::Phi), 0, "{}", graph.dump());
    // x == 0 was undefined; every other input keeps its result.
    for input in [1, 2, u64::MAX] {
        assert_eq!(
            execute(&original, input),
            execute(&graph, input),
            "input {input}"
        );
    }
}

#[test]
fn an_observable_operation_before_the_undefined_entry_is_kept() {
    let mut graph = fixture(true);
    verify_graph(&mut graph).unwrap();
    let original = graph.clone();
    run_final(&mut graph).unwrap();
    verify_graph(&mut graph).unwrap_or_else(|error| panic!("{error}\n{}", graph.dump()));
    // The inner branch folds; the outer one still selects the call path.
    assert_eq!(count(&graph, Opcode::If), 1, "{}", graph.dump());
    assert_eq!(count(&graph, Opcode::Call), 1, "{}", graph.dump());
    for input in [0, 1, 2, u64::MAX] {
        assert_eq!(
            execute(&original, input),
            execute(&graph, input),
            "input {input}"
        );
    }
}

#[test]
fn ordinary_simplification_keeps_branches_into_unreachable() {
    let mut graph = fixture(false);
    verify_graph(&mut graph).unwrap();
    run(&mut graph).unwrap();
    // Diamond folding may select the merged value, but the branch into
    // Unreachable keeps serving as a fact until the final cleanup.
    assert!(count(&graph, Opcode::If) >= 1, "{}", graph.dump());
    assert_eq!(count(&graph, Opcode::Unreachable), 1, "{}", graph.dump());
}
