use kfrc_body::{
    FunctionAbi, IntegerWidth, Mutability, PassMode, RecordedTypes, SizeAlign, TyId, TypeShape,
};
use kfrc_son::{
    CallTarget, ComparePredicate, FunctionGraph, InterpretError, Interpreter, NodeId, Opcode,
    SemanticCallHandler, SemanticMemory, SemanticOutcome, Value, ValueType,
    optimize_specialized_graph, optimize_specialized_seams, verify_graph,
};

struct Fixture {
    graph: FunctionGraph,
    boolean: TyId,
    word: TyId,
    control: NodeId,
    effect: NodeId,
    args: [NodeId; 3],
}

impl Fixture {
    fn new() -> Self {
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
            "conditions",
            FunctionAbi {
                args: vec![PassMode::Direct; 3],
                ret: PassMode::Direct,
                c_variadic: false,
            },
            types,
        );
        let start = graph.add_node(
            Opcode::Start,
            NodeId::NONE,
            &[],
            0,
            ValueType::Tuple(vec![
                ValueType::Control,
                ValueType::Effect,
                ValueType::Rust(word),
                ValueType::Rust(word),
                ValueType::Rust(boolean),
            ]),
        );
        graph.start = start;
        let mut projection =
            |slot, ty| graph.add_node(Opcode::Projection, NodeId::NONE, &[start], slot, ty);
        let control = projection(0, ValueType::Control);
        let effect = projection(1, ValueType::Effect);
        let args = [
            projection(2, ValueType::Rust(word)),
            projection(3, ValueType::Rust(word)),
            projection(4, ValueType::Rust(boolean)),
        ];
        Self {
            graph,
            boolean,
            word,
            control,
            effect,
            args,
        }
    }
    fn project(&mut self, source: NodeId, slot: u32, ty: ValueType) -> NodeId {
        self.graph
            .add_node(Opcode::Projection, NodeId::NONE, &[source], slot, ty)
    }
    fn branch(&mut self, at: NodeId, condition: NodeId) -> [NodeId; 2] {
        let branch = self.graph.add_node(
            Opcode::If,
            at,
            &[condition],
            0,
            ValueType::Tuple(vec![ValueType::Control; 2]),
        );
        [0, 1].map(|index| self.project(branch, index, ValueType::Control))
    }
    fn ret(&mut self, control: NodeId, effect: NodeId, value: u128) {
        let value = self.graph.add_constant(value, self.word);
        self.graph.add_node(
            Opcode::Return,
            control,
            &[effect, value],
            0,
            ValueType::Control,
        );
    }
    fn not(&mut self, value: NodeId) -> NodeId {
        self.graph.add_node(
            Opcode::BitNot,
            NodeId::NONE,
            &[value],
            0,
            ValueType::Rust(self.boolean),
        )
    }
    fn less(&mut self, left: NodeId, right: NodeId) -> NodeId {
        self.graph.add_node(
            Opcode::Compare,
            NodeId::NONE,
            &[left, right],
            ComparePredicate::UnsignedLess as u32,
            ValueType::Rust(self.boolean),
        )
    }
    fn assume(&mut self, at: NodeId, condition: NodeId) -> (NodeId, NodeId) {
        let assume = self.graph.add_node(
            Opcode::Assume,
            at,
            &[self.effect, condition],
            0,
            ValueType::Tuple(vec![ValueType::Control, ValueType::Effect]),
        );
        (
            self.project(assume, 0, ValueType::Control),
            self.project(assume, 1, ValueType::Effect),
        )
    }
    fn jump(&mut self, at: NodeId) -> NodeId {
        self.graph
            .add_node(Opcode::Jump, at, &[], 0, ValueType::Control)
    }
    fn fixed(
        &mut self,
        opcode: Opcode,
        operands: &[NodeId],
        payload: u32,
        result: Option<TyId>,
    ) -> Option<NodeId> {
        let mut inputs = vec![self.effect];
        inputs.extend_from_slice(operands);
        let mut outputs = vec![ValueType::Control, ValueType::Effect];
        if let Some(ty) = result {
            outputs.push(ValueType::Rust(ty));
        }
        let node = self.graph.add_node(
            opcode,
            self.control,
            &inputs,
            payload,
            ValueType::Tuple(outputs),
        );
        self.control = self.project(node, 0, ValueType::Control);
        self.effect = self.project(node, 1, ValueType::Effect);
        result.map(|ty| self.project(node, 2, ValueType::Rust(ty)))
    }
}

fn run(graph: &FunctionGraph, args: [u64; 3]) -> Result<SemanticOutcome, InterpretError> {
    let mut interpreter = Interpreter::new(graph);
    interpreter.fuel = 200;
    interpreter.run_values(
        &args.map(Value::Scalar),
        &mut |symbol: &str, _: &[Value]| Err(InterpretError::UnknownCallee(symbol.into())),
    )
}

fn differential(f: &mut Fixture, args: &[[u64; 3]], expected_folds: usize, seam: bool) {
    verify_graph(&mut f.graph).unwrap();
    let before: Vec<_> = args.iter().map(|args| run(&f.graph, *args)).collect();
    let stats = if seam {
        optimize_specialized_seams(&mut f.graph, &[], &[]).unwrap()
    } else {
        optimize_specialized_graph(&mut f.graph).unwrap()
    };
    assert_eq!(stats.conditions_folded, expected_folds, "{stats:?}");
    #[cfg(debug_assertions)]
    assert_eq!(stats.condition_receipts_checked, expected_folds);
    verify_graph(&mut f.graph).unwrap();
    for (args, before) in args.iter().zip(before) {
        assert_eq!(run(&f.graph, *args), before, "{args:?}");
    }
}

#[test]
fn repeated_predicates_and_boolean_negations_fold_on_both_exits() {
    for seam in [false, true] {
        for negate_proof in [false, true] {
            let mut f = Fixture::new();
            let condition = f.less(f.args[0], f.args[1]);
            let proof_condition = if negate_proof {
                f.not(condition)
            } else {
                condition
            };
            let exits = f.branch(f.control, proof_condition);
            for (i, at) in exits.into_iter().enumerate() {
                let negated = f.not(condition);
                let [yes, no] = f.branch(at, negated);
                f.ret(yes, f.effect, 10 + i as u128);
                f.ret(no, f.effect, 20 + i as u128);
            }
            differential(&mut f, &[[0, 1, 0], [1, 1, 0], [u64::MAX, 0, 1]], 2, seam);
        }
    }
}

#[test]
fn assume_proves_only_its_continuation_and_never_an_earlier_use() {
    let mut f = Fixture::new();
    let [yes, no] = f.branch(f.control, f.args[2]);
    let left = f.jump(yes);
    let right = f.jump(no);
    let merge = f.graph.add_node(
        Opcode::Region,
        NodeId::NONE,
        &[left, right],
        0,
        ValueType::Control,
    );
    let (after, effect) = f.assume(merge, f.args[2]);
    let negated = f.not(f.args[2]);
    let [impossible, normal] = f.branch(after, negated);
    f.ret(impossible, effect, 99);
    f.ret(normal, effect, 7);
    differential(&mut f, &[[0, 0, 1], [12, 17, 1]], 1, false);
    assert_eq!(
        f.graph
            .ids()
            .filter(|&n| f.graph.op(n) == Opcode::If)
            .count(),
        1
    );
    assert_eq!(
        f.graph
            .ids()
            .filter(|&n| f.graph.op(n) == Opcode::Assume)
            .count(),
        1
    );
}

#[test]
fn a_sibling_branch_fact_does_not_escape_the_join() {
    let mut f = Fixture::new();
    let [left, right] = f.branch(f.control, f.args[2]);
    let condition = f.less(f.args[0], f.args[1]);
    let [yes, no] = f.branch(left, condition);
    let jumps = [yes, no, right].map(|at| f.jump(at));
    let merge = f
        .graph
        .add_node(Opcode::Region, NodeId::NONE, &jumps, 0, ValueType::Control);
    let [yes, no] = f.branch(merge, condition);
    f.ret(yes, f.effect, 1);
    f.ret(no, f.effect, 2);
    differential(
        &mut f,
        &[[0, 1, 0], [0, 1, 1], [1, 0, 0], [1, 0, 1]],
        0,
        false,
    );
}

#[test]
fn a_loop_guard_refreshes_its_predicate_each_iteration() {
    let mut f = Fixture::new();
    let entry = f.jump(f.control);
    let header = f.graph.add_node(
        Opcode::Region,
        NodeId::NONE,
        &[entry],
        0,
        ValueType::Control,
    );
    let zero = f.graph.add_constant(0, f.word);
    let i = f.graph.add_node(
        Opcode::Phi,
        NodeId::NONE,
        &[header, zero],
        0,
        ValueType::Rust(f.word),
    );
    let condition = f.less(i, f.args[0]);
    let [body, exit] = f.branch(header, condition);
    f.ret(exit, f.effect, 7);
    let [again, impossible] = f.branch(body, condition);
    f.ret(impossible, f.effect, 99);
    let one = f.graph.add_constant(1, f.word);
    let next = f.graph.add_node(
        Opcode::Add,
        NodeId::NONE,
        &[i, one],
        0,
        ValueType::Rust(f.word),
    );
    let back = f.jump(again);
    f.graph.set_inputs(header, &[entry, back]);
    f.graph.set_inputs(i, &[header, zero, next]);
    differential(&mut f, &[[0, 0, 0], [1, 0, 0], [5, 0, 0]], 1, false);
}

#[test]
fn an_assumption_from_the_previous_iteration_does_not_prove_the_next_guard() {
    let mut f = Fixture::new();
    let entry = f.jump(f.control);
    let header = f.graph.add_node(
        Opcode::Region,
        NodeId::NONE,
        &[entry],
        0,
        ValueType::Control,
    );
    let yes_value = f.graph.add_constant(1, f.boolean);
    let no_value = f.graph.add_constant(0, f.boolean);
    let predicate = f.graph.add_node(
        Opcode::Phi,
        NodeId::NONE,
        &[header, yes_value],
        0,
        ValueType::Rust(f.boolean),
    );
    let [body, exit] = f.branch(header, predicate);
    f.ret(exit, f.effect, 7);
    let (after, next_effect) = f.assume(body, predicate);
    let back = f.jump(after);
    f.graph.set_inputs(header, &[entry, back]);
    f.graph
        .set_inputs(predicate, &[header, yes_value, no_value]);
    let effect = f.graph.add_node(
        Opcode::Phi,
        NodeId::NONE,
        &[header, f.effect, next_effect],
        0,
        ValueType::Effect,
    );
    // The assumption's effect belongs to this iteration; its predicate does
    // not become a fact at the loop header before the assumption executes.
    let assume = f.graph.input(after, 0);
    f.graph.set_input(assume, 0, effect);
    let ret = f
        .graph
        .ids()
        .find(|&n| f.graph.op(n) == Opcode::Return)
        .unwrap();
    f.graph.set_input(ret, 0, effect);
    differential(&mut f, &[[0, 0, 0]], 0, false);
    assert_eq!(
        run(&f.graph, [0, 0, 0]),
        Ok(SemanticOutcome::Return(Value::Scalar(7)))
    );
}

struct LengthWriter;
impl SemanticCallHandler for LengthWriter {
    fn call(&mut self, symbol: &str, _: &[Value]) -> Result<Value, InterpretError> {
        Err(InterpretError::UnknownCallee(symbol.into()))
    }

    fn call_with_memory(
        &mut self,
        _symbol: &str,
        args: &[Value],
        memory: &mut SemanticMemory,
    ) -> Result<Value, InterpretError> {
        let Value::Scalar(value) = args[1] else {
            unreachable!()
        };
        let Value::Scalar(address) = args[0] else {
            unreachable!()
        };
        memory.write_bytes(address, &value.to_le_bytes().map(Some))?;
        Ok(Value::Scalar(0))
    }
}

#[test]
fn reloading_length_after_a_mutating_call_does_not_reuse_the_old_predicate() {
    let mut f = Fixture::new();
    let pointer = f.graph.oracle.intern(
        TypeShape::RawPointer {
            mutability: Mutability::Mut,
            pointee: f.word,
        },
        Some(SizeAlign { size: 8, align: 8 }),
    );
    let slot = f.graph.add_node(
        Opcode::LocalSlot,
        NodeId::NONE,
        &[],
        0,
        ValueType::Rust(pointer),
    );
    let target = f.graph.add_callee(CallTarget {
        symbol: "write_length".into(),
        abi: FunctionAbi {
            args: vec![PassMode::Direct; 2],
            ret: PassMode::Direct,
            c_variadic: false,
        },
    });
    f.fixed(Opcode::Call, &[slot, f.args[1]], target, Some(f.word));
    let first = f
        .fixed(Opcode::LoadValue, &[slot], 0, Some(f.word))
        .unwrap();
    let before_condition = f.less(f.args[0], first);
    let [body, no] = f.branch(f.control, before_condition);
    f.ret(no, f.effect, 9);
    f.control = body;
    let zero = f.graph.add_constant(0, f.word);
    f.fixed(Opcode::Call, &[slot, zero], target, Some(f.word));
    let second = f
        .fixed(Opcode::LoadValue, &[slot], 0, Some(f.word))
        .unwrap();
    let condition = f.less(f.args[0], second);
    let [yes, no] = f.branch(f.control, condition);
    f.ret(yes, f.effect, 99);
    f.ret(no, f.effect, 7);
    let execute = |graph: &FunctionGraph, args: [u64; 3]| {
        Interpreter::new(graph).run_values(&args.map(Value::Scalar), &mut LengthWriter)
    };
    verify_graph(&mut f.graph).unwrap();
    let inputs = [[0, 1, 0], [4, 7, 0], [4, 2, 0]];
    let expected = inputs.map(|args| execute(&f.graph, args));
    assert_eq!(expected[0], Ok(SemanticOutcome::Return(Value::Scalar(7))));
    let stats = optimize_specialized_graph(&mut f.graph).unwrap();
    assert_eq!(stats.conditions_folded, 0);
    verify_graph(&mut f.graph).unwrap();
    for (args, expected) in inputs.into_iter().zip(expected) {
        assert_eq!(execute(&f.graph, args), expected);
    }
}

#[test]
fn a_fold_keeps_the_original_poison_observation() {
    let mut f = Fixture::new();
    let poison = f.graph.add_node(
        Opcode::Undefined,
        NodeId::NONE,
        &[],
        0,
        ValueType::Rust(f.boolean),
    );
    let [outer_yes, outer_no] = f.branch(f.control, poison);
    f.ret(outer_no, f.effect, 3);
    let [inner_yes, inner_no] = f.branch(outer_yes, poison);
    f.ret(inner_yes, f.effect, 1);
    f.ret(inner_no, f.effect, 2);
    verify_graph(&mut f.graph).unwrap();
    assert!(matches!(
        run(&f.graph, [0, 0, 0]),
        Err(InterpretError::PoisonObserved(_))
    ));
    let stats = optimize_specialized_graph(&mut f.graph).unwrap();
    assert_eq!(stats.conditions_folded, 1);
    verify_graph(&mut f.graph).unwrap();
    assert!(matches!(
        run(&f.graph, [0, 0, 0]),
        Err(InterpretError::PoisonObserved(_))
    ));
}

#[path = "semantic_conditions/implication.rs"]
mod implication_tests;
