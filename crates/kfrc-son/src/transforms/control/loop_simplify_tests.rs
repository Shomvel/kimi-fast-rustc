use super::*;
use crate::{
    ComparePredicate, Interpreter, SemanticOutcome, Value, ValueType, optimize_specialized_graph,
    optimize_specialized_seams, verify_graph,
};
use kfrc_body::{FunctionAbi, IntegerWidth, PassMode, RecordedTypes, SizeAlign, TyId, TypeShape};

struct Fixture {
    graph: FunctionGraph,
    word: TyId,
    header: NodeId,
    iv: NodeId,
    accumulator: NodeId,
    effect: NodeId,
    entries: [NodeId; 2],
    entry_values: [NodeId; 2],
    entry_effects: [NodeId; 2],
    latch: NodeId,
    body_observation: NodeId,
}

fn project(graph: &mut FunctionGraph, parent: NodeId, index: u32, ty: ValueType) -> NodeId {
    graph.add_node(Opcode::Projection, NodeId::NONE, &[parent], index, ty)
}

fn observe(
    graph: &mut FunctionGraph,
    control: NodeId,
    effect: NodeId,
    value: NodeId,
    word: TyId,
) -> (NodeId, NodeId, NodeId, NodeId) {
    let source = graph.add_node(
        Opcode::Opaque,
        control,
        &[effect, value],
        0,
        ValueType::Tuple(vec![
            ValueType::Control,
            ValueType::Effect,
            ValueType::Rust(word),
        ]),
    );
    (
        project(graph, source, 0, ValueType::Control),
        project(graph, source, 1, ValueType::Effect),
        project(graph, source, 2, ValueType::Rust(word)),
        source,
    )
}

impl Fixture {
    fn new(initial: u64, limit: u64) -> Self {
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
            "loop_preheader",
            FunctionAbi {
                args: vec![PassMode::Direct; 2],
                ret: PassMode::Direct,
                c_variadic: false,
            },
            types,
        );
        graph.level = GraphLevel::Specialized;
        let start = graph.add_node(
            Opcode::Start,
            NodeId::NONE,
            &[],
            0,
            ValueType::Tuple(vec![
                ValueType::Control,
                ValueType::Effect,
                ValueType::Rust(boolean),
                ValueType::Rust(word),
            ]),
        );
        graph.start = start;
        let control = project(&mut graph, start, 0, ValueType::Control);
        let effect = project(&mut graph, start, 1, ValueType::Effect);
        let condition = project(&mut graph, start, 2, ValueType::Rust(boolean));
        let input = project(&mut graph, start, 3, ValueType::Rust(word));
        let entry_if = graph.add_node(
            Opcode::If,
            control,
            &[condition],
            0,
            ValueType::Tuple(vec![ValueType::Control; 2]),
        );
        let mut entries = Vec::new();
        let mut entry_values = Vec::new();
        let mut entry_effects = Vec::new();
        for (side, delta) in [(0, 3), (1, 9)] {
            let control = project(&mut graph, entry_if, side, ValueType::Control);
            let delta = graph.add_constant(delta, word);
            let value = graph.add_node(
                Opcode::Add,
                NodeId::NONE,
                &[input, delta],
                0,
                ValueType::Rust(word),
            );
            let (control, effect, value, _) = observe(&mut graph, control, effect, value, word);
            entries.push(graph.add_node(Opcode::Jump, control, &[], 0, ValueType::Control));
            entry_values.push(value);
            entry_effects.push(effect);
        }
        let header = graph.add_node(
            Opcode::Region,
            NodeId::NONE,
            &entries,
            0,
            ValueType::Control,
        );
        let initial = graph.add_constant(u128::from(initial), word);
        let bound = graph.add_constant(u128::from(limit), word);
        let one = graph.add_constant(1, word);
        let iv = graph.add_node(
            Opcode::Phi,
            NodeId::NONE,
            &[header, initial, initial],
            0,
            ValueType::Rust(word),
        );
        let accumulator = graph.add_node(
            Opcode::Phi,
            NodeId::NONE,
            &[header, entry_values[0], entry_values[1]],
            0,
            ValueType::Rust(word),
        );
        let loop_effect = graph.add_node(
            Opcode::Phi,
            NodeId::NONE,
            &[header, entry_effects[0], entry_effects[1]],
            0,
            ValueType::Effect,
        );
        let condition = graph.add_node(
            Opcode::Compare,
            NodeId::NONE,
            &[iv, bound],
            ComparePredicate::UnsignedLess as u32,
            ValueType::Rust(boolean),
        );
        let branch = graph.add_node(
            Opcode::If,
            header,
            &[condition],
            0,
            ValueType::Tuple(vec![ValueType::Control; 2]),
        );
        let body = project(&mut graph, branch, 0, ValueType::Control);
        let exit = project(&mut graph, branch, 1, ValueType::Control);
        let (body, back_effect, observed, body_observation) =
            observe(&mut graph, body, loop_effect, accumulator, word);
        let next = graph.add_node(
            Opcode::Add,
            NodeId::NONE,
            &[iv, one],
            0,
            ValueType::Rust(word),
        );
        let updated = graph.add_node(
            Opcode::Add,
            NodeId::NONE,
            &[observed, iv],
            0,
            ValueType::Rust(word),
        );
        let latch = graph.add_node(Opcode::Jump, body, &[], 0, ValueType::Control);
        // Interleave entry/latch/entry to make slot order part of the contract.
        graph.set_inputs(header, &[entries[0], latch, entries[1]]);
        graph.set_inputs(iv, &[header, initial, next, initial]);
        graph.set_inputs(
            accumulator,
            &[header, entry_values[0], updated, entry_values[1]],
        );
        graph.set_inputs(
            loop_effect,
            &[header, entry_effects[0], back_effect, entry_effects[1]],
        );
        let result = graph.add_node(
            Opcode::BitXor,
            NodeId::NONE,
            &[accumulator, iv],
            0,
            ValueType::Rust(word),
        );
        graph.add_node(
            Opcode::Return,
            exit,
            &[loop_effect, result],
            0,
            ValueType::Control,
        );
        verify_graph(&mut graph).unwrap();
        Self {
            graph,
            word,
            header,
            iv,
            accumulator,
            effect: loop_effect,
            entries: entries.try_into().unwrap(),
            entry_values: entry_values.try_into().unwrap(),
            entry_effects: entry_effects.try_into().unwrap(),
            latch,
            body_observation,
        }
    }
}

fn execute(graph: &FunctionGraph, condition: bool, value: u64) -> SemanticOutcome {
    Interpreter::new(graph)
        .run_values(
            &[Value::Scalar(u64::from(condition)), Value::Scalar(value)],
            &mut |_: &str, _: &[Value]| unreachable!(),
        )
        .unwrap()
}

fn equivalent(before: &FunctionGraph, after: &FunctionGraph) {
    for condition in [false, true] {
        for value in [0, 1, 127, 1 << 63, u64::MAX] {
            assert_eq!(
                execute(before, condition, value),
                execute(after, condition, value)
            );
        }
    }
}

#[test]
fn coalesces_entry_slots_and_value_effect_phis_together() {
    let mut f = Fixture::new(0, 4);
    let before = f.graph.clone();
    let stats = run(&mut f.graph).unwrap();
    assert_eq!(stats.preheaders_created, 1);
    assert_eq!(stats.phis_created, 2); // IV has the same entry value on both edges.
    assert_eq!(stats.nodes_added, 4);
    verify_graph(&mut f.graph).unwrap();
    assert_eq!(f.graph.inputs(f.header).len(), 2);
    assert_eq!(f.graph.input(f.header, 1), f.latch);
    let preheader = f.graph.control(f.graph.input(f.header, 0));
    assert_eq!(f.graph.inputs(preheader), &f.entries);
    for (phi, entries) in [(f.accumulator, f.entry_values), (f.effect, f.entry_effects)] {
        let grouped = f.graph.input(phi, 1);
        assert_eq!(
            f.graph.inputs(grouped),
            &[preheader, entries[0], entries[1]]
        );
    }
    assert_eq!(f.graph.op(f.graph.input(f.iv, 1)), Opcode::Const);
    equivalent(&before, &f.graph);
    let dump = f.graph.dump();
    assert_eq!(run(&mut f.graph).unwrap().preheaders_created, 0);
    verify_graph(&mut f.graph).unwrap();
    assert_eq!(f.graph.dump(), dump);
}

#[test]
fn preserves_reversed_predecessor_and_latch_slots() {
    let mut f = Fixture::new(0, 4);
    let phis: Vec<_> = f
        .graph
        .ids()
        .filter(|&node| f.graph.op(node) == Opcode::Phi && f.graph.input(node, 0) == f.header)
        .collect();
    for phi in phis {
        let inputs = f.graph.inputs(phi).to_vec();
        f.graph
            .set_inputs(phi, &[f.header, inputs[2], inputs[3], inputs[1]]);
    }
    f.graph
        .set_inputs(f.header, &[f.latch, f.entries[1], f.entries[0]]);
    verify_graph(&mut f.graph).unwrap();
    let before = f.graph.clone();
    assert_eq!(run(&mut f.graph).unwrap().preheaders_created, 1);
    verify_graph(&mut f.graph).unwrap();
    assert_eq!(f.graph.input(f.header, 0), f.latch);
    let preheader = f.graph.control(f.graph.input(f.header, 1));
    assert_eq!(f.graph.inputs(preheader), &[f.entries[1], f.entries[0]]);
    equivalent(&before, &f.graph);
}

#[test]
fn zero_trip_body_poison_stays_unexecuted() {
    let mut f = Fixture::new(4, 4);
    let poison = f.graph.add_node(
        Opcode::Undefined,
        NodeId::NONE,
        &[],
        0,
        ValueType::Rust(f.word),
    );
    f.graph.set_input(f.body_observation, 1, poison);
    verify_graph(&mut f.graph).unwrap();
    let before = f.graph.clone();
    assert_eq!(run(&mut f.graph).unwrap().preheaders_created, 1);
    verify_graph(&mut f.graph).unwrap();
    equivalent(&before, &f.graph);
}

#[test]
fn full_and_seam_pipelines_unroll_after_entry_coalescing() {
    for full in [true, false] {
        let mut f = Fixture::new(0, 4);
        let before = f.graph.clone();
        let stats = if full {
            optimize_specialized_graph(&mut f.graph).unwrap()
        } else {
            optimize_specialized_seams(&mut f.graph, &[], &[]).unwrap()
        };
        assert_eq!(stats.loop_preheaders_created, 1);
        assert_eq!(stats.loops_unrolled, 1);
        assert_eq!(stats.loop_iterations_cloned, 4);
        verify_graph(&mut f.graph).unwrap();
        equivalent(&before, &f.graph);
    }
}

#[test]
fn budgets_and_other_graph_levels_leave_the_graph_intact() {
    let f = Fixture::new(0, 4);
    for limits in [
        Limits {
            analysis: 1,
            ..Limits::default()
        },
        Limits {
            preheaders: 0,
            ..Limits::default()
        },
        Limits {
            nodes: 1,
            ..Limits::default()
        },
        Limits {
            phis: 0,
            ..Limits::default()
        },
        Limits {
            entries: 1,
            ..Limits::default()
        },
    ] {
        let mut graph = f.graph.clone();
        let before = graph.dump();
        let stats = run_with_limits(&mut graph, limits).unwrap();
        assert_eq!(stats.preheaders_created, 0);
        assert!(stats.budget_bailouts > 0);
        assert!(stats.analysis_visits <= limits.analysis);
        verify_graph(&mut graph).unwrap();
        assert_eq!(graph.dump(), before);
    }
    for level in [GraphLevel::Rust, GraphLevel::Layout] {
        let mut graph = f.graph.clone();
        graph.level = level;
        let before = graph.dump();
        assert_eq!(run(&mut graph).unwrap(), LoopSimplifyStats::default());
        assert_eq!(graph.dump(), before);
    }
}

#[test]
fn malformed_phi_is_rejected_before_grouping_publishes_nodes() {
    let mut f = Fixture::new(0, 4);
    let inputs = f.graph.inputs(f.effect).to_vec();
    f.graph.set_inputs(f.effect, &inputs[..3]);
    let before = f.graph.dump();
    assert_eq!(
        CfgEditor::new(&mut f.graph).group_predecessors(f.header, &[true, false, true]),
        Err(CfgEditError::MalformedPhi {
            region: f.header,
            phi: f.effect
        }),
    );
    assert_eq!(f.graph.dump(), before);
}

#[test]
fn multiple_latches_keep_their_original_entry_structure() {
    let mut f = Fixture::new(0, 4);
    let condition = f.graph.find_projection(f.graph.start, 2).unwrap();
    let branch = f.graph.add_node(
        Opcode::If,
        f.graph.control(f.latch),
        &[condition],
        0,
        ValueType::Tuple(vec![ValueType::Control; 2]),
    );
    let yes = project(&mut f.graph, branch, 0, ValueType::Control);
    let no = project(&mut f.graph, branch, 1, ValueType::Control);
    f.graph.set_control(f.latch, yes);
    let second_latch = f
        .graph
        .add_node(Opcode::Jump, no, &[], 0, ValueType::Control);
    let phis: Vec<_> = f
        .graph
        .ids()
        .filter(|&node| f.graph.op(node) == Opcode::Phi && f.graph.input(node, 0) == f.header)
        .collect();
    for phi in phis {
        let mut inputs = f.graph.inputs(phi).to_vec();
        inputs.push(inputs[2]);
        f.graph.set_inputs(phi, &inputs);
    }
    f.graph.set_inputs(
        f.header,
        &[f.entries[0], f.latch, f.entries[1], second_latch],
    );
    verify_graph(&mut f.graph).unwrap();
    let before = f.graph.dump();
    assert_eq!(run(&mut f.graph).unwrap().preheaders_created, 0);
    verify_graph(&mut f.graph).unwrap();
    assert_eq!(f.graph.dump(), before);
}
