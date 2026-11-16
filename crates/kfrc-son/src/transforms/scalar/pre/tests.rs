use super::*;
use crate::{
    ArithmeticFlags, ComparePredicate, InterpretError, Interpreter, Opcode, SemanticOutcome, Value,
    ValueType, verify_graph,
};
use kfrc_body::{FunctionAbi, IntegerWidth, PassMode, RecordedTypes, SizeAlign, TyId, TypeShape};

#[path = "tests/rejections.rs"]
mod rejections;

struct Fixture {
    graph: FunctionGraph,
    word: TyId,
    boolean: TyId,
    control: NodeId,
    effect: NodeId,
    args: [NodeId; 3],
    region: NodeId,
    preds: [NodeId; 2],
    phi: NodeId,
    delta: NodeId,
    existing: [NodeId; 2],
    result: NodeId,
    ret: NodeId,
}

fn projection(graph: &mut FunctionGraph, parent: NodeId, slot: u32, ty: ValueType) -> NodeId {
    graph.add_node(Opcode::Projection, NodeId::NONE, &[parent], slot, ty)
}

impl Fixture {
    fn new(op: Opcode, observed: [bool; 2], swapped: bool) -> Self {
        let mut types = RecordedTypes::new();
        let word = types.intern(
            TypeShape::Int {
                width: IntegerWidth::W32,
                signed: false,
            },
            Some(SizeAlign { size: 4, align: 4 }),
        );
        let boolean = types.intern(TypeShape::Bool, Some(SizeAlign { size: 1, align: 1 }));
        let mut graph = FunctionGraph::new(
            "scalar_pre",
            FunctionAbi {
                args: vec![PassMode::Direct; 3],
                ret: PassMode::Direct,
                c_variadic: false,
            },
            types,
        );
        graph.level = GraphLevel::Specialized;
        graph.start = graph.add_node(
            Opcode::Start,
            NodeId::NONE,
            &[],
            0,
            ValueType::Tuple(vec![
                ValueType::Control,
                ValueType::Effect,
                ValueType::Rust(boolean),
                ValueType::Rust(word),
                ValueType::Rust(word),
            ]),
        );
        let start = graph.start;
        let control = projection(&mut graph, start, 0, ValueType::Control);
        let effect = projection(&mut graph, start, 1, ValueType::Effect);
        let condition = projection(&mut graph, start, 2, ValueType::Rust(boolean));
        let x = projection(&mut graph, start, 3, ValueType::Rust(word));
        let y = projection(&mut graph, start, 4, ValueType::Rust(word));
        let branch = graph.add_node(
            Opcode::If,
            control,
            &[condition],
            0,
            ValueType::Tuple(vec![ValueType::Control; 2]),
        );
        let delta = graph.add_constant(17, word);
        let mut existing = [NodeId::NONE; 2];
        let mut preds = [NodeId::NONE; 2];
        let mut effects = [effect; 2];
        for (side, input) in [x, y].into_iter().enumerate() {
            let mut at = projection(&mut graph, branch, side as u32, ValueType::Control);
            existing[side] = graph.add_node(
                op,
                NodeId::NONE,
                &if swapped {
                    [delta, input]
                } else {
                    [input, delta]
                },
                0,
                ValueType::Rust(word),
            );
            if observed[side] {
                let observe = graph.add_node(
                    Opcode::RequireDefined,
                    at,
                    &[effect, existing[side]],
                    0,
                    ValueType::Tuple(vec![ValueType::Control, ValueType::Effect]),
                );
                at = projection(&mut graph, observe, 0, ValueType::Control);
                effects[side] = projection(&mut graph, observe, 1, ValueType::Effect);
            }
            preds[side] = graph.add_node(Opcode::Jump, at, &[], 0, ValueType::Control);
        }
        let region = graph.add_node(Opcode::Region, NodeId::NONE, &preds, 0, ValueType::Control);
        let effect_phi = graph.add_node(
            Opcode::Phi,
            NodeId::NONE,
            &[region, effects[0], effects[1]],
            0,
            ValueType::Effect,
        );
        let phi = graph.add_node(
            Opcode::Phi,
            NodeId::NONE,
            &[region, x, y],
            0,
            ValueType::Rust(word),
        );
        let result = graph.add_node(op, NodeId::NONE, &[phi, delta], 0, ValueType::Rust(word));
        let ret = graph.add_node(
            Opcode::Return,
            region,
            &[effect_phi, result],
            0,
            ValueType::Control,
        );
        Self {
            graph,
            word,
            boolean,
            control,
            effect,
            args: [condition, x, y],
            region,
            preds,
            phi,
            delta,
            existing,
            result,
            ret,
        }
    }

    fn verify(&mut self) {
        verify_graph(&mut self.graph).unwrap();
    }

    fn observe_at_entry(&mut self, value: NodeId) {
        let observe = self.graph.add_node(
            Opcode::RequireDefined,
            self.control,
            &[self.effect, value],
            0,
            ValueType::Tuple(vec![ValueType::Control, ValueType::Effect]),
        );
        let continuation = projection(&mut self.graph, observe, 0, ValueType::Control);
        let effect = projection(&mut self.graph, observe, 1, ValueType::Effect);
        let users: Vec<_> = self
            .graph
            .ids()
            .filter(|&node| node != observe && self.graph.control(node) == self.control)
            .collect();
        for user in users {
            self.graph.set_control(user, continuation);
        }
        let consumers: Vec<_> = self
            .graph
            .ids()
            .filter(|&node| {
                node != observe
                    && self.graph.op(node).is_fixed()
                    && self.graph.inputs(node).first() == Some(&self.effect)
            })
            .collect();
        for consumer in consumers {
            self.graph.set_input(consumer, 0, effect);
        }
        let effects: Vec<_> = self
            .graph
            .ids()
            .filter(|&node| {
                self.graph.op(node) == Opcode::Phi
                    && self.graph.value_type(node) == &ValueType::Effect
            })
            .collect();
        for phi in effects {
            let inputs = self.graph.inputs(phi).to_vec();
            for (slot, input) in inputs.into_iter().enumerate().skip(1) {
                if input == self.effect {
                    self.graph.set_input(phi, slot, effect);
                }
            }
        }
    }
}

fn execute(graph: &FunctionGraph, args: &[Value]) -> Result<SemanticOutcome, InterpretError> {
    Interpreter::new(graph).run_values(args, &mut |_: &str, _: &[Value]| unreachable!())
}

fn equivalent(before: &FunctionGraph, after: &FunctionGraph) {
    for side in [0, 1] {
        for value in [
            0,
            1,
            255,
            65535,
            1 << 31,
            u32::MAX as u64 - 16,
            u32::MAX as u64,
        ] {
            let args = [
                Value::Scalar(side),
                Value::Scalar(value),
                Value::Scalar(value ^ 0x1234),
            ];
            assert_eq!(execute(before, &args), execute(after, &args));
        }
        for args in [
            [Value::Scalar(side), Value::Poison, Value::Scalar(31)],
            [Value::Scalar(side), Value::Scalar(31), Value::Poison],
            [Value::Poison, Value::Scalar(31), Value::Scalar(31)],
        ] {
            assert_eq!(
                execute(before, &args).is_err(),
                execute(after, &args).is_err()
            );
        }
    }
}

fn no_change(mut f: Fixture, limits: Limits) -> PreStats {
    f.verify();
    let before = f.graph.dump();
    let stats = run_with_limits(&mut f.graph, limits);
    assert_eq!(stats.expressions_replaced, 0);
    assert_eq!(before, f.graph.dump());
    stats
}

#[test]
fn one_available_predecessor_inserts_exactly_one_expression_and_one_phi() {
    let mut f = Fixture::new(Opcode::Add, [true, false], false);
    f.verify();
    let before = f.graph.clone();
    let old_len = f.graph.len();
    let stats = run(&mut f.graph);
    assert_eq!(stats.expressions_replaced, 1);
    assert_eq!(stats.expressions_inserted, 1);
    assert_eq!(stats.phis_created, 1);
    assert_eq!(f.graph.len() - old_len, 2);
    let result = f.graph.resolve(f.result);
    assert_eq!(f.graph.op(result), Opcode::Phi);
    assert_eq!(f.graph.input(result, 1), f.existing[0]);
    assert!(stats.dirty_nodes.contains(&f.ret));
    f.verify();
    equivalent(&before, &f.graph);
}

#[test]
fn both_available_predecessors_need_no_new_arithmetic_and_commute_exactly() {
    for op in [
        Opcode::Add,
        Opcode::Mul,
        Opcode::BitAnd,
        Opcode::BitOr,
        Opcode::BitXor,
    ] {
        let mut f = Fixture::new(op, [true, true], true);
        f.verify();
        let before = f.graph.clone();
        let stats = run(&mut f.graph);
        assert_eq!(stats.expressions_replaced, 1);
        assert_eq!(stats.expressions_inserted, 0);
        assert_eq!(stats.phis_created, 1);
        f.verify();
        equivalent(&before, &f.graph);
    }
}

#[test]
fn every_direct_phi_operand_is_translated_and_other_uses_keep_their_value() {
    let mut f = Fixture::new(Opcode::Mul, [true, true], false);
    let second = f.graph.add_node(
        Opcode::Phi,
        NodeId::NONE,
        &[f.region, f.delta, f.args[1]],
        0,
        ValueType::Rust(f.word),
    );
    f.graph.set_input(f.result, 1, second);
    f.graph.set_inputs(f.existing[1], &[f.args[2], f.args[1]]);
    let tuple = f
        .graph
        .oracle
        .intern(TypeShape::Tuple(vec![f.word, f.word]), None);
    let output = f.graph.add_node(
        Opcode::Aggregate,
        NodeId::NONE,
        &[f.result, f.phi],
        0,
        ValueType::Rust(tuple),
    );
    f.graph.set_input(f.ret, 1, output);
    f.verify();
    let before = f.graph.clone();
    assert_eq!(run(&mut f.graph).expressions_replaced, 1);
    assert!(!f.graph.is_dead(f.phi));
    f.verify();
    equivalent(&before, &f.graph);
}

#[test]
fn a_live_phi_incoming_edge_is_real_execution_evidence() {
    let mut f = Fixture::new(Opcode::Add, [false, false], false);
    let saved = f.graph.add_node(
        Opcode::Phi,
        NodeId::NONE,
        &[f.region, f.existing[0], f.args[2]],
        0,
        ValueType::Rust(f.word),
    );
    let tuple = f
        .graph
        .oracle
        .intern(TypeShape::Tuple(vec![f.word, f.word]), None);
    let output = f.graph.add_node(
        Opcode::Aggregate,
        NodeId::NONE,
        &[f.result, saved],
        0,
        ValueType::Rust(tuple),
    );
    f.graph.set_input(f.ret, 1, output);
    f.verify();
    let before = f.graph.clone();
    assert_eq!(run(&mut f.graph).expressions_replaced, 1);
    f.verify();
    equivalent(&before, &f.graph);
}

#[test]
fn several_plans_keep_the_original_node_ids_until_the_batch_is_published() {
    let mut f = Fixture::new(Opcode::Add, [true, false], false);
    let existing = f.graph.add_node(
        Opcode::BitXor,
        NodeId::NONE,
        &[f.args[1], f.delta],
        0,
        ValueType::Rust(f.word),
    );
    f.observe_at_entry(existing);
    let second = f.graph.add_node(
        Opcode::BitXor,
        NodeId::NONE,
        &[f.phi, f.delta],
        0,
        ValueType::Rust(f.word),
    );
    let tuple = f
        .graph
        .oracle
        .intern(TypeShape::Tuple(vec![f.word; 2]), None);
    let output = f.graph.add_node(
        Opcode::Aggregate,
        NodeId::NONE,
        &[f.result, second],
        0,
        ValueType::Rust(tuple),
    );
    f.graph.set_input(f.ret, 1, output);
    f.verify();
    let before = f.graph.clone();
    let stats = run(&mut f.graph);
    assert_eq!(stats.expressions_replaced, 2);
    assert_eq!(stats.expressions_inserted, 2);
    assert_eq!(stats.phis_created, 2);
    f.verify();
    equivalent(&before, &f.graph);
}

#[test]
fn a_branch_result_is_available_only_after_its_own_definition() {
    let mut f = Fixture::new(Opcode::Add, [true, false], false);
    let observe = f.graph.input(f.graph.control(f.preds[0]), 0);
    let opaque = f.graph.add_node(
        Opcode::Opaque,
        f.graph.control(observe),
        &[f.effect, f.args[1]],
        0,
        ValueType::Tuple(vec![
            ValueType::Control,
            ValueType::Effect,
            ValueType::Rust(f.word),
        ]),
    );
    let control = projection(&mut f.graph, opaque, 0, ValueType::Control);
    let effect = projection(&mut f.graph, opaque, 1, ValueType::Effect);
    let value = projection(&mut f.graph, opaque, 2, ValueType::Rust(f.word));
    f.graph.set_control(observe, control);
    f.graph.set_input(observe, 0, effect);
    f.graph.set_input(f.existing[0], 0, value);
    f.graph.set_input(f.phi, 1, value);
    f.verify();
    let before = f.graph.clone();
    assert_eq!(run(&mut f.graph).expressions_replaced, 1);
    f.verify();
    equivalent(&before, &f.graph);
}

#[test]
fn full_and_seeded_pipelines_really_trigger_pre() {
    for seeded in [false, true] {
        // A pre-branch use remains live after observation elimination. A
        // nontrivial If consumes the expression, so it is real work on both
        // predecessor paths and GVN cannot equate op(phi(x,y),17) with it.
        let mut f = Fixture::new(Opcode::Add, [false, false], false);
        let value = f.existing[0];
        let zero = f.graph.add_constant(0, f.word);
        let predicate = f.graph.add_node(
            Opcode::Compare,
            NodeId::NONE,
            &[value, zero],
            ComparePredicate::NotEqual as u32,
            ValueType::Rust(f.boolean),
        );
        let branch = f
            .graph
            .ids()
            .find(|&n| f.graph.op(n) == Opcode::If)
            .unwrap();
        f.graph.set_input(branch, 0, predicate);
        f.verify();
        let before = f.graph.clone();
        let stats = if seeded {
            crate::optimize_specialized_seams(&mut f.graph, &[f.result], &[]).unwrap()
        } else {
            crate::optimize_specialized_graph(&mut f.graph).unwrap()
        };
        assert!(stats.pre_expressions_replaced > 0);
        assert!(
            f.graph.ids().any(|n| f.graph.op(n) == Opcode::Phi
                && f.graph.value_type(n) == &ValueType::Rust(f.word))
        );
        f.verify();
        equivalent(&before, &f.graph);
    }
}
