use kfrc_body::{FunctionAbi, IntegerWidth, PassMode, RecordedTypes, SizeAlign, TyId, TypeShape};
use kfrc_son::{
    ArithmeticFlags, CallTarget, ComparePredicate, FunctionGraph, GraphLevel, Interpreter, NodeId,
    Opcode, SemanticOutcome, Value, ValueType, optimize_specialized_graph,
    optimize_specialized_seams, verify_graph,
};

struct Fixture {
    g: FunctionGraph,
    word: TyId,
    boolean: TyId,
    control: NodeId,
    effect: NodeId,
    condition: NodeId,
    argument: NodeId,
}

impl Fixture {
    fn new(width: IntegerWidth) -> Self {
        let mut types = RecordedTypes::new();
        let bytes = u64::from(width.bits(64)) / 8;
        let word = types.intern(
            TypeShape::Int {
                width,
                signed: false,
            },
            Some(SizeAlign {
                size: bytes,
                align: bytes,
            }),
        );
        let boolean = types.intern(TypeShape::Bool, Some(SizeAlign { size: 1, align: 1 }));
        let mut g = FunctionGraph::new(
            "sccp",
            FunctionAbi {
                args: vec![PassMode::Direct; 2],
                ret: PassMode::Direct,
                c_variadic: false,
            },
            types,
        );
        g.level = GraphLevel::Specialized;
        g.start = g.add_node(
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
        let mut project = |i, ty| g.add_node(Opcode::Projection, NodeId::NONE, &[g.start], i, ty);
        let control = project(0, ValueType::Control);
        let effect = project(1, ValueType::Effect);
        let condition = project(2, ValueType::Rust(boolean));
        let argument = project(3, ValueType::Rust(word));
        Self {
            g,
            word,
            boolean,
            control,
            effect,
            condition,
            argument,
        }
    }
    fn c(&mut self, bits: u128) -> NodeId {
        self.g.add_constant(bits, self.word)
    }
    fn binary(&mut self, op: Opcode, a: NodeId, b: NodeId, payload: u32) -> NodeId {
        self.g.add_node(
            op,
            NodeId::NONE,
            &[a, b],
            payload,
            ValueType::Rust(if op == Opcode::Compare {
                self.boolean
            } else {
                self.word
            }),
        )
    }
    fn branch(&mut self, control: NodeId, condition: NodeId) -> (NodeId, NodeId) {
        let n = self.g.add_node(
            Opcode::If,
            control,
            &[condition],
            0,
            ValueType::Tuple(vec![ValueType::Control; 2]),
        );
        let a = self.g.add_node(
            Opcode::Projection,
            NodeId::NONE,
            &[n],
            0,
            ValueType::Control,
        );
        let b = self.g.add_node(
            Opcode::Projection,
            NodeId::NONE,
            &[n],
            1,
            ValueType::Control,
        );
        (a, b)
    }
    fn jump(&mut self, c: NodeId) -> NodeId {
        self.g.add_node(Opcode::Jump, c, &[], 0, ValueType::Control)
    }
    fn merge(&mut self, a: NodeId, b: NodeId, x: NodeId, y: NodeId) -> (NodeId, NodeId) {
        let a = self.jump(a);
        let b = self.jump(b);
        let region = self
            .g
            .add_node(Opcode::Region, NodeId::NONE, &[a, b], 0, ValueType::Control);
        let phi = self.g.add_node(
            Opcode::Phi,
            NodeId::NONE,
            &[region, x, y],
            0,
            ValueType::Rust(self.word),
        );
        (region, phi)
    }
    fn ret(&mut self, c: NodeId, e: NodeId, v: NodeId) {
        self.g
            .add_node(Opcode::Return, c, &[e, v], 0, ValueType::Control);
    }
    fn call(&mut self, c: NodeId, name: &str) -> (NodeId, NodeId) {
        let target = self.g.add_callee(CallTarget {
            symbol: name.into(),
            abi: FunctionAbi {
                args: vec![],
                ret: PassMode::Direct,
                c_variadic: false,
            },
        });
        let call = self.g.add_node(
            Opcode::Call,
            c,
            &[self.effect],
            target,
            ValueType::Tuple(vec![
                ValueType::Control,
                ValueType::Effect,
                ValueType::Rust(self.word),
            ]),
        );
        let c = self.g.add_node(
            Opcode::Projection,
            NodeId::NONE,
            &[call],
            0,
            ValueType::Control,
        );
        let e = self.g.add_node(
            Opcode::Projection,
            NodeId::NONE,
            &[call],
            1,
            ValueType::Effect,
        );
        (c, e)
    }
}

fn execute(
    g: &FunctionGraph,
    condition: Value,
    argument: u64,
) -> (
    Result<SemanticOutcome, kfrc_son::InterpretError>,
    Vec<String>,
) {
    let mut trace = Vec::new();
    let mut interpreter = Interpreter::new(g);
    interpreter.fuel = 1000;
    let result = interpreter.run_values(
        &[condition, Value::Scalar(argument)],
        &mut |name: &str, _: &[Value]| {
            trace.push(name.to_owned());
            Ok(Value::Scalar(17))
        },
    );
    (result, trace)
}

#[test]
fn joint_propagation_resolves_a_long_chain_but_keeps_selector_poison() {
    for seams in [false, true] {
        let mut f = Fixture::new(IntegerWidth::W64);
        let (a, b) = f.branch(f.control, f.condition);
        let seven = f.c(7);
        let (mut control, mut value) = f.merge(a, b, seven, seven);
        let wrong = f.c(99);
        for _ in 0..12 {
            let equal = f.binary(
                Opcode::Compare,
                value,
                seven,
                ComparePredicate::Equal as u32,
            );
            let (a, b) = f.branch(control, equal);
            (control, value) = f.merge(a, b, seven, wrong);
        }
        f.ret(control, f.effect, value);
        verify_graph(&mut f.g).unwrap();
        let original = f.g.clone();
        let stats = if seams {
            optimize_specialized_seams(&mut f.g, &[], &[]).unwrap()
        } else {
            optimize_specialized_graph(&mut f.g).unwrap()
        };
        if !seams {
            assert!(stats.sccp_constants > 0);
        }
        assert_eq!(stats.sccp_budget_bailouts, 0);
        assert_eq!(f.g.ids().filter(|&n| f.g.op(n) == Opcode::If).count(), 1);
        for c in [Value::Scalar(0), Value::Scalar(1), Value::Poison] {
            let before = execute(&original, c.clone(), 0);
            let after = execute(&f.g, c, 0);
            assert_eq!(before.0.is_err(), after.0.is_err());
            if before.0.is_ok() {
                assert_eq!(before, after);
            }
        }
    }
}

#[test]
fn reachable_poison_and_overflow_contracts_are_not_constant_inputs() {
    for flagged in [false, true] {
        let mut f = Fixture::new(IntegerWidth::W8);
        let (a, b) = f.branch(f.control, f.condition);
        let seven = f.c(7);
        let bad = if flagged {
            let max = f.c(255);
            let one = f.c(1);
            f.binary(Opcode::Add, max, one, ArithmeticFlags::NUW)
        } else {
            f.g.add_node(
                Opcode::Undefined,
                NodeId::NONE,
                &[],
                0,
                ValueType::Rust(f.word),
            )
        };
        let (region, phi) = f.merge(a, b, seven, bad);
        f.ret(region, f.effect, phi);
        verify_graph(&mut f.g).unwrap();
        let original = f.g.clone();
        optimize_specialized_graph(&mut f.g).unwrap();
        assert_eq!(
            execute(&f.g, Value::Scalar(1), 0),
            execute(&original, Value::Scalar(1), 0)
        );
        assert!(execute(&original, Value::Scalar(0), 0).0.is_err());
        assert!(execute(&f.g, Value::Scalar(0), 0).0.is_err());
    }
}

#[test]
fn constant_phi_prunes_only_the_dead_call_and_updates_effect_phis() {
    let mut f = Fixture::new(IntegerWidth::W64);
    let (a, b) = f.branch(f.control, f.condition);
    let seven = f.c(7);
    let (region, phi) = f.merge(a, b, seven, seven);
    let equal = f.binary(Opcode::Compare, phi, seven, ComparePredicate::Equal as u32);
    let (a, b) = f.branch(region, equal);
    let (a, ea) = f.call(a, "retained");
    let (b, eb) = f.call(b, "removed");
    let bad = f.c(99);
    let (region, value) = f.merge(a, b, seven, bad);
    let effect = f.g.add_node(
        Opcode::Phi,
        NodeId::NONE,
        &[region, ea, eb],
        0,
        ValueType::Effect,
    );
    f.ret(region, effect, value);
    verify_graph(&mut f.g).unwrap();
    let original = f.g.clone();
    optimize_specialized_graph(&mut f.g).unwrap();
    for c in [0, 1] {
        assert_eq!(
            execute(&f.g, Value::Scalar(c), 0),
            execute(&original, Value::Scalar(c), 0)
        );
    }
    assert_eq!(execute(&f.g, Value::Scalar(0), 0).1, ["retained"]);
    assert_eq!(f.g.ids().filter(|&n| f.g.op(n) == Opcode::Call).count(), 1);
}

fn loop_fixture(infinite: bool) -> Fixture {
    let mut f = Fixture::new(IntegerWidth::W64);
    let entry = f.jump(f.control);
    let region =
        f.g.add_growable_node(Opcode::Region, NodeId::NONE, 2, 0, ValueType::Control);
    f.g.set_input(region, 0, entry);
    let i =
        f.g.add_growable_node(Opcode::Phi, NodeId::NONE, 3, 0, ValueType::Rust(f.word));
    let invariant =
        f.g.add_growable_node(Opcode::Phi, NodeId::NONE, 3, 0, ValueType::Rust(f.word));
    let zero = f.c(0);
    let one = f.c(1);
    let answer = f.c(42);
    f.g.set_inputs(i, &[region, zero, NodeId::NONE]);
    f.g.set_inputs(invariant, &[region, answer, invariant]);
    let condition = if infinite {
        f.g.add_constant(1, f.boolean)
    } else {
        f.binary(
            Opcode::Compare,
            i,
            f.argument,
            ComparePredicate::UnsignedLess as u32,
        )
    };
    let (body, exit) = f.branch(region, condition);
    let next = f.binary(Opcode::Add, i, one, 0);
    f.g.set_input(i, 2, next);
    let back = f.jump(body);
    f.g.set_input(region, 1, back);
    let result = f.binary(Opcode::Add, i, invariant, 0);
    f.ret(exit, f.effect, result);
    f
}

#[test]
fn loop_phi_widens_on_a_late_backedge_while_invariants_stay_constant() {
    let mut f = loop_fixture(false);
    verify_graph(&mut f.g).unwrap();
    let original = f.g.clone();
    let stats = optimize_specialized_graph(&mut f.g).unwrap();
    assert!(stats.sccp_constants > 0);
    for limit in [0, 1, 2, 7, 31] {
        let actual = execute(&f.g, Value::Scalar(0), limit);
        assert_eq!(actual, execute(&original, Value::Scalar(0), limit));
        assert_eq!(
            actual.0.unwrap(),
            SemanticOutcome::Return(Value::Scalar(limit + 42))
        );
    }
}

#[test]
fn a_live_nonterminating_loop_is_not_deleted() {
    let mut f = loop_fixture(true);
    verify_graph(&mut f.g).unwrap();
    let original = f.g.clone();
    optimize_specialized_graph(&mut f.g).unwrap();
    assert!(execute(&original, Value::Scalar(0), 0).0.is_err());
    assert!(execute(&f.g, Value::Scalar(0), 0).0.is_err());
    assert!(f.g.ids().any(|n| f.g.op(n) == Opcode::Region));
    assert!(!f.g.ids().any(|n| f.g.op(n) == Opcode::Return));
}

#[test]
fn narrow_constants_wrap_at_the_source_width() {
    for seams in [false, true] {
        let mut f = Fixture::new(IntegerWidth::W8);
        let (a, b) = f.branch(f.control, f.condition);
        let max = f.c(255);
        let (region, phi) = f.merge(a, b, max, max);
        let two = f.c(2);
        let wrapped = f.binary(Opcode::Add, phi, two, 0);
        f.ret(region, f.effect, wrapped);
        verify_graph(&mut f.g).unwrap();
        let original = f.g.clone();
        let stats = if seams {
            optimize_specialized_seams(&mut f.g, &[], &[]).unwrap()
        } else {
            optimize_specialized_graph(&mut f.g).unwrap()
        };
        assert!(stats.sccp_constants >= 2);
        for c in [0, 1] {
            assert_eq!(
                execute(&f.g, Value::Scalar(c), 0),
                execute(&original, Value::Scalar(c), 0)
            );
            assert_eq!(
                execute(&f.g, Value::Scalar(c), 0).0.unwrap(),
                SemanticOutcome::Return(Value::Scalar(1))
            );
        }
    }
}
