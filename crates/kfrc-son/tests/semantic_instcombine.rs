use kfrc_body::{FunctionAbi, IntegerWidth, PassMode, RecordedTypes, SizeAlign, TyId, TypeShape};
use kfrc_son::{
    ArithmeticFlags, FunctionGraph, GraphLevel, Interpreter, NodeId, Opcode, SemanticOutcome,
    Value, ValueType, optimize_graph, optimize_specialized_seams, verify_graph,
};

struct Function {
    graph: FunctionGraph,
    ty: TyId,
    args: Vec<NodeId>,
    control: NodeId,
    effect: NodeId,
}

impl Function {
    fn new(width: IntegerWidth, signed: bool, count: usize) -> Self {
        let mut types = RecordedTypes::new();
        let bytes = u64::from(width.bits(64)) / 8;
        let ty = types.intern(
            TypeShape::Int { width, signed },
            Some(SizeAlign {
                size: bytes,
                align: bytes,
            }),
        );
        let mut graph = FunctionGraph::new(
            "combine",
            FunctionAbi {
                args: vec![PassMode::Direct; count],
                ret: PassMode::Direct,
                c_variadic: false,
            },
            types,
        );
        graph.level = GraphLevel::Specialized;
        let mut outputs = vec![ValueType::Control, ValueType::Effect];
        outputs.extend(vec![ValueType::Rust(ty); count]);
        graph.start = graph.add_node(
            Opcode::Start,
            NodeId::NONE,
            &[],
            0,
            ValueType::Tuple(outputs),
        );
        let mut projection =
            |index, ty| graph.add_node(Opcode::Projection, NodeId::NONE, &[graph.start], index, ty);
        let control = projection(0, ValueType::Control);
        let effect = projection(1, ValueType::Effect);
        let args = (0..count)
            .map(|i| projection(i as u32 + 2, ValueType::Rust(ty)))
            .collect();
        Self {
            graph,
            ty,
            args,
            control,
            effect,
        }
    }

    fn constant(&mut self, n: u64) -> NodeId {
        self.graph.add_constant(u128::from(n), self.ty)
    }

    fn binary(&mut self, op: Opcode, a: NodeId, b: NodeId, flags: u32) -> NodeId {
        self.graph
            .add_node(op, NodeId::NONE, &[a, b], flags, ValueType::Rust(self.ty))
    }

    fn finish(&mut self, result: NodeId) {
        self.graph.add_node(
            Opcode::Return,
            self.control,
            &[self.effect, result],
            0,
            ValueType::Control,
        );
        verify_graph(&mut self.graph).unwrap();
    }
}

fn run(graph: &FunctionGraph, args: &[Value]) -> Result<SemanticOutcome, kfrc_son::InterpretError> {
    Interpreter::new(graph).run_values(args, &mut |_: &str, _: &[Value]| unreachable!())
}

fn count(graph: &FunctionGraph, op: Opcode) -> usize {
    graph.ids().filter(|&n| graph.op(n) == op).count()
}

#[test]
fn reversed_commutative_expressions_share_one_value() {
    for op in [
        Opcode::Add,
        Opcode::Mul,
        Opcode::BitAnd,
        Opcode::BitOr,
        Opcode::BitXor,
    ] {
        let mut f = Function::new(IntegerWidth::W64, false, 2);
        let a = f.binary(op, f.args[0], f.args[1], 0);
        let b = f.binary(op, f.args[1], f.args[0], 0);
        let result = f.binary(Opcode::Sub, a, b, 0);
        f.finish(result);
        let before = f.graph.clone();
        optimize_graph(&mut f.graph);
        verify_graph(&mut f.graph).unwrap();
        assert_eq!(count(&f.graph, op), 1, "{op:?}");
        for (a, b) in [(0, 1), (u64::MAX, 17), (1 << 63, 0x1234)] {
            let args = [Value::Scalar(a), Value::Scalar(b)];
            assert_eq!(run(&f.graph, &args), run(&before, &args));
        }
        let args = [Value::Poison, Value::Scalar(3)];
        assert!(run(&before, &args).is_err());
        assert!(run(&f.graph, &args).is_err());
    }
}

#[test]
fn nested_constants_combine_at_the_actual_integer_width() {
    for width in [
        IntegerWidth::W8,
        IntegerWidth::W16,
        IntegerWidth::W32,
        IntegerWidth::W64,
    ] {
        let mask = u64::MAX >> (64 - width.bits(64));
        for signed in [false, true] {
            for op in [
                Opcode::Add,
                Opcode::Mul,
                Opcode::BitAnd,
                Opcode::BitOr,
                Opcode::BitXor,
            ] {
                for reversed in [false, true] {
                    let mut f = Function::new(width, signed, 1);
                    let a = f.constant(mask - 1);
                    let b = f.constant(3);
                    let inner = if reversed {
                        f.binary(op, a, f.args[0], 0)
                    } else {
                        f.binary(op, f.args[0], a, 0)
                    };
                    let outer = if reversed {
                        f.binary(op, b, inner, 0)
                    } else {
                        f.binary(op, inner, b, 0)
                    };
                    f.finish(outer);
                    let before = f.graph.clone();
                    optimize_graph(&mut f.graph);
                    verify_graph(&mut f.graph).unwrap();
                    assert!(count(&f.graph, op) <= 1, "{width:?} {signed} {op:?}");
                    for value in [0, 1, 2, 17, mask / 2, mask - 1, mask] {
                        let args = [Value::Scalar(value)];
                        assert_eq!(run(&f.graph, &args), run(&before, &args));
                    }
                    assert!(run(&before, &[Value::Poison]).is_err());
                    assert!(run(&f.graph, &[Value::Poison]).is_err());
                    let once = f.graph.dump();
                    optimize_graph(&mut f.graph);
                    verify_graph(&mut f.graph).unwrap();
                    assert_eq!(once, f.graph.dump(), "rewrite must converge");
                }
            }
        }
    }
}

#[test]
fn reassociation_keeps_shared_inner_consumers_intact() {
    let mut f = Function::new(IntegerWidth::W8, false, 1);
    let first = f.constant(253);
    let second = f.constant(7);
    let inner = f.binary(Opcode::Add, f.args[0], first, 0);
    let outer = f.binary(Opcode::Add, inner, second, 0);
    let result = f.binary(Opcode::BitXor, inner, outer, 0);
    f.finish(result);
    let before = f.graph.clone();
    optimize_graph(&mut f.graph);
    verify_graph(&mut f.graph).unwrap();
    for input in 0..=255 {
        let args = [Value::Scalar(input)];
        assert_eq!(run(&f.graph, &args), run(&before, &args));
    }
}

#[test]
fn arithmetic_and_disjoint_contracts_block_reassociation() {
    for (op, flag) in [
        (Opcode::Add, ArithmeticFlags::NUW),
        (Opcode::Add, ArithmeticFlags::NSW),
        (Opcode::Mul, ArithmeticFlags::NUW),
        (Opcode::BitOr, ArithmeticFlags::DISJOINT),
    ] {
        for inner_flagged in [false, true] {
            let mut f = Function::new(IntegerWidth::W8, false, 1);
            let a = f.constant(127);
            let b = f.constant(3);
            let inner = f.binary(op, f.args[0], a, if inner_flagged { flag } else { 0 });
            let outer = f.binary(op, inner, b, if inner_flagged { 0 } else { flag });
            f.finish(outer);
            let before = f.graph.clone();
            optimize_graph(&mut f.graph);
            verify_graph(&mut f.graph).unwrap();
            assert_eq!(count(&f.graph, op), 2);
            for input in 0..=255 {
                let args = [Value::Scalar(input)];
                assert_eq!(run(&f.graph, &args), run(&before, &args));
            }
        }
    }
}

#[test]
fn a_zero_combined_constant_does_not_hide_poison() {
    for (op, a, b) in [(Opcode::Mul, 16, 16), (Opcode::BitAnd, 15, 240)] {
        let mut f = Function::new(IntegerWidth::W8, false, 1);
        let a = f.constant(a);
        let b = f.constant(b);
        let inner = f.binary(op, f.args[0], a, 0);
        let outer = f.binary(op, inner, b, 0);
        f.finish(outer);
        optimize_graph(&mut f.graph);
        verify_graph(&mut f.graph).unwrap();
        assert_eq!(count(&f.graph, op), 1);
        assert!(run(&f.graph, &[Value::Poison]).is_err());
        assert_eq!(
            run(&f.graph, &[Value::Scalar(42)]).unwrap(),
            SemanticOutcome::Return(Value::Scalar(0))
        );
    }
}

#[test]
fn splice_seams_apply_the_same_combination_rules() {
    let mut f = Function::new(IntegerWidth::W64, false, 1);
    let a = f.constant(5);
    let b = f.constant(7);
    let inner = f.binary(Opcode::Add, f.args[0], a, 0);
    let outer = f.binary(Opcode::Add, inner, b, 0);
    f.finish(outer);
    optimize_specialized_seams(&mut f.graph, &[outer], &[]).unwrap();
    verify_graph(&mut f.graph).unwrap();
    assert_eq!(count(&f.graph, Opcode::Add), 1);
    assert_eq!(
        run(&f.graph, &[Value::Scalar(30)]).unwrap(),
        SemanticOutcome::Return(Value::Scalar(42))
    );
}
