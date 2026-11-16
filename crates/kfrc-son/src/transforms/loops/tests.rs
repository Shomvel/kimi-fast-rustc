use super::*;
use crate::{
    ArithmeticFlags, ComparePredicate, InterpretError, Interpreter, SemanticOutcome, Value,
    ValueType, verify_graph,
};
use kfrc_body::{FunctionAbi, IntegerWidth, PassMode, RecordedTypes, SizeAlign, TyId, TypeShape};

#[path = "tests/effects.rs"]
mod effects;

#[path = "tests/conditions.rs"]
mod conditions;

#[path = "tests/modular.rs"]
mod modular;

#[path = "tests/observations.rs"]
mod observations;

#[path = "tests/strength_reduce.rs"]
mod strength_reduction;

struct Fixture {
    graph: FunctionGraph,
    word: TyId,
    boolean: TyId,
    header: NodeId,
    branch: NodeId,
    body: NodeId,
    exit: NodeId,
    latch: NodeId,
    iv: NodeId,
    a: NodeId,
    b: NodeId,
    effect: NodeId,
    condition: NodeId,
    next_iv: NodeId,
    next_a: NodeId,
    ret: NodeId,
}

fn projection(graph: &mut FunctionGraph, parent: NodeId, slot: u32, ty: ValueType) -> NodeId {
    graph.add_node(Opcode::Projection, NodeId::NONE, &[parent], slot, ty)
}

impl Fixture {
    fn new(width: IntegerWidth, signed: bool, start: u64, end: u64, step: u64) -> Self {
        let mut types = RecordedTypes::new();
        let size = u64::from(width.bits(64)) / 8;
        let word = types.intern(
            TypeShape::Int { width, signed },
            Some(SizeAlign { size, align: size }),
        );
        let boolean = types.intern(TypeShape::Bool, Some(SizeAlign { size: 1, align: 1 }));
        let tuple = types.intern(TypeShape::Tuple(vec![word; 3]), None);
        let mut graph = FunctionGraph::new(
            "unroll",
            FunctionAbi {
                args: vec![PassMode::Direct; 2],
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
                ValueType::Rust(word),
                ValueType::Rust(word),
            ]),
        );
        let source = graph.start;
        let entry_control = projection(&mut graph, source, 0, ValueType::Control);
        let entry_effect = projection(&mut graph, source, 1, ValueType::Effect);
        let x = projection(&mut graph, source, 2, ValueType::Rust(word));
        let y = projection(&mut graph, source, 3, ValueType::Rust(word));
        let entry = graph.add_node(Opcode::Jump, entry_control, &[], 0, ValueType::Control);
        let header = graph.add_node(
            Opcode::Region,
            NodeId::NONE,
            &[entry],
            0,
            ValueType::Control,
        );
        let initial = graph.add_constant(u128::from(start), word);
        let limit = graph.add_constant(u128::from(end), word);
        let increment = graph.add_constant(u128::from(step), word);
        let iv = graph.add_node(
            Opcode::Phi,
            NodeId::NONE,
            &[header, initial],
            0,
            ValueType::Rust(word),
        );
        let a = graph.add_node(
            Opcode::Phi,
            NodeId::NONE,
            &[header, x],
            0,
            ValueType::Rust(word),
        );
        let b = graph.add_node(
            Opcode::Phi,
            NodeId::NONE,
            &[header, y],
            0,
            ValueType::Rust(word),
        );
        let effect = graph.add_node(
            Opcode::Phi,
            NodeId::NONE,
            &[header, entry_effect],
            0,
            ValueType::Effect,
        );
        let condition = graph.add_node(
            Opcode::Compare,
            NodeId::NONE,
            &[iv, limit],
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
        let body = projection(&mut graph, branch, 0, ValueType::Control);
        let exit = projection(&mut graph, branch, 1, ValueType::Control);
        let next_iv = graph.add_node(
            Opcode::Add,
            NodeId::NONE,
            &[iv, increment],
            0,
            ValueType::Rust(word),
        );
        let next_a = graph.add_node(
            Opcode::Add,
            NodeId::NONE,
            &[b, iv],
            0,
            ValueType::Rust(word),
        );
        let mix = graph.add_constant(0x5a, word);
        let next_b = graph.add_node(
            Opcode::BitXor,
            NodeId::NONE,
            &[a, mix],
            0,
            ValueType::Rust(word),
        );
        let latch = graph.add_node(Opcode::Jump, body, &[], 0, ValueType::Control);
        graph.set_inputs(header, &[entry, latch]);
        graph.set_inputs(iv, &[header, initial, next_iv]);
        graph.set_inputs(a, &[header, x, next_a]);
        graph.set_inputs(b, &[header, y, next_b]);
        graph.set_inputs(effect, &[header, entry_effect, effect]);
        let output = graph.add_node(
            Opcode::Aggregate,
            NodeId::NONE,
            &[iv, a, b],
            0,
            ValueType::Rust(tuple),
        );
        let ret = graph.add_node(
            Opcode::Return,
            exit,
            &[effect, output],
            0,
            ValueType::Control,
        );
        Self {
            graph,
            word,
            boolean,
            header,
            branch,
            body,
            exit,
            latch,
            iv,
            a,
            b,
            effect,
            condition,
            next_iv,
            next_a,
            ret,
        }
    }

    fn verify(&mut self) {
        verify_graph(&mut self.graph).unwrap();
    }

    fn fixed(&mut self, op: Opcode, value: NodeId, returns_value: bool) -> Option<NodeId> {
        let mut results = vec![ValueType::Control, ValueType::Effect];
        if returns_value {
            results.push(ValueType::Rust(self.word));
        }
        let node = self.graph.add_node(
            op,
            self.body,
            &[self.effect, value],
            0,
            ValueType::Tuple(results),
        );
        self.body = projection(&mut self.graph, node, 0, ValueType::Control);
        self.effect = projection(&mut self.graph, node, 1, ValueType::Effect);
        self.graph.set_control(self.latch, self.body);
        let effect_phi = self.graph.input(self.ret, 0);
        self.graph.set_input(effect_phi, 2, self.effect);
        returns_value.then(|| projection(&mut self.graph, node, 2, ValueType::Rust(self.word)))
    }
}

fn execute(graph: &FunctionGraph, args: &[Value]) -> Result<SemanticOutcome, InterpretError> {
    Interpreter::new(graph).run_values(args, &mut |_: &str, _: &[Value]| unreachable!())
}

fn equivalent(before: &FunctionGraph, after: &FunctionGraph, mask: u64) {
    for value in [0, 1, 127, mask / 2, mask - 1, mask] {
        let args = [
            Value::Scalar(value & mask),
            Value::Scalar((value ^ 0x1234) & mask),
        ];
        let a = execute(before, &args);
        let b = execute(after, &args);
        assert_eq!(a.is_ok(), b.is_ok(), "{value:#x}");
        if a.is_ok() {
            assert_eq!(a, b, "{value:#x}");
        }
    }
    for args in [
        [Value::Poison, Value::Scalar(1)],
        [Value::Scalar(1), Value::Poison],
    ] {
        assert_eq!(
            execute(before, &args).is_err(),
            execute(after, &args).is_err()
        );
    }
}

fn unchanged(mut f: Fixture, limits: Limits) -> UnrollStats {
    f.verify();
    let before = f.graph.dump();
    let stats = run_with_limits(&mut f.graph, limits).unwrap();
    assert_eq!(stats.loops_unrolled, 0);
    assert_eq!(
        before,
        f.graph.dump(),
        "rejected plans must leave the graph untouched"
    );
    stats
}

#[test]
fn counted_loops_unroll_zero_one_and_eight_iterations_with_simultaneous_phis() {
    for width in [IntegerWidth::W8, IntegerWidth::W32, IntegerWidth::W64] {
        for count in [0, 1, 4, 8] {
            let mut f = Fixture::new(width, false, 0, count, 1);
            f.verify();
            let before = f.graph.clone();
            let old_len = f.graph.len();
            let stats = run(&mut f.graph).unwrap();
            assert_eq!(stats.loops_unrolled, 1);
            assert_eq!(stats.iterations_cloned, count as usize);
            assert_eq!(stats.nodes_cloned, f.graph.len() - old_len);
            assert!(!f.graph.ids().any(|n| f.graph.op(n) == Opcode::If));
            f.verify();
            equivalent(&before, &f.graph, u64::MAX >> (64 - width.bits(64)));
            assert_eq!(run(&mut f.graph).unwrap().loops_unrolled, 0);
        }
    }
}

#[test]
fn predecessor_slots_and_branch_polarity_are_not_assumed() {
    let mut f = Fixture::new(IntegerWidth::W32, false, 0, 4, 1);
    let old = f.graph.inputs(f.header).to_vec();
    f.graph.set_inputs(f.header, &[old[1], old[0]]);
    for phi in [f.iv, f.a, f.b, f.effect] {
        let inputs = f.graph.inputs(phi).to_vec();
        f.graph.set_inputs(phi, &[inputs[0], inputs[2], inputs[1]]);
    }
    f.graph.set_payload(f.body, 1);
    f.graph.set_payload(f.exit, 0);
    f.graph
        .set_payload(f.condition, ComparePredicate::UnsignedGreaterEqual as u32);
    f.verify();
    let before = f.graph.clone();
    assert_eq!(run(&mut f.graph).unwrap().iterations_cloned, 4);
    f.verify();
    equivalent(&before, &f.graph, u32::MAX.into());
}

#[test]
fn signed_descending_and_swapped_comparisons_use_exact_values() {
    let mut f = Fixture::new(IntegerWidth::W8, true, 3, 255, 255);
    f.graph
        .set_payload(f.condition, ComparePredicate::SignedLess as u32);
    let inputs = f.graph.inputs(f.condition).to_vec();
    f.graph.set_inputs(f.condition, &[inputs[1], inputs[0]]);
    f.verify();
    let before = f.graph.clone();
    assert_eq!(run(&mut f.graph).unwrap().iterations_cloned, 4);
    f.verify();
    equivalent(&before, &f.graph, 255);
}

#[test]
fn promised_induction_and_unknown_trip_counts_are_rejected() {
    let mut f = Fixture::new(IntegerWidth::W32, false, 0, 4, 1);
    f.graph.set_payload(f.next_iv, ArithmeticFlags::NUW);
    unchanged(f, Limits::default());
    let mut f = Fixture::new(IntegerWidth::W32, false, 0, 4, 1);
    f.graph.set_input(f.condition, 1, f.a);
    unchanged(f, Limits::default());
}

#[test]
fn all_growth_and_analysis_budgets_reject_before_cloning() {
    for limits in [
        Limits {
            iterations: 3,
            ..Limits::default()
        },
        Limits {
            template_nodes: 1,
            ..Limits::default()
        },
        Limits {
            loop_nodes: 1,
            ..Limits::default()
        },
        Limits {
            total_nodes: 1,
            ..Limits::default()
        },
        Limits {
            loops: 0,
            ..Limits::default()
        },
        Limits {
            analysis: 1,
            ..Limits::default()
        },
    ] {
        assert!(
            unchanged(Fixture::new(IntegerWidth::W32, false, 0, 4, 1), limits).budget_bailouts > 0
        );
    }
}

#[test]
fn extra_exit_and_anchored_traps_are_not_silently_removed() {
    let mut f = Fixture::new(IntegerWidth::W32, false, 0, 4, 1);
    let extra = f.graph.add_node(
        Opcode::If,
        f.body,
        &[f.condition],
        0,
        ValueType::Tuple(vec![ValueType::Control; 2]),
    );
    let yes = projection(&mut f.graph, extra, 0, ValueType::Control);
    let no = projection(&mut f.graph, extra, 1, ValueType::Control);
    f.graph.set_control(f.latch, yes);
    f.graph.add_node(
        Opcode::Return,
        no,
        &[f.effect, f.graph.input(f.ret, 1)],
        0,
        ValueType::Control,
    );
    unchanged(f, Limits::default());
    let mut f = Fixture::new(IntegerWidth::W32, false, 0, 4, 1);
    let zero = f.graph.add_constant(0, f.word);
    f.graph.add_node(
        Opcode::DivUnsigned,
        f.body,
        &[f.a, zero],
        0,
        ValueType::Rust(f.word),
    );
    unchanged(f, Limits::default());
}

#[test]
fn only_specialized_graphs_are_considered() {
    for level in [GraphLevel::Rust, GraphLevel::Layout] {
        let mut f = Fixture::new(IntegerWidth::W32, false, 0, 4, 1);
        f.graph.level = level;
        let before = f.graph.dump();
        assert_eq!(run(&mut f.graph).unwrap(), UnrollStats::default());
        assert_eq!(before, f.graph.dump());
    }
}
