//! Differential proof of power-of-two division across widths and trap paths.
use kfrc_body::{FunctionAbi, IntegerWidth, PassMode, RecordedTypes, SizeAlign, TyId, TypeShape};
use kfrc_son::{
    CallTarget, FunctionGraph, GraphLevel, InterpretError, Interpreter, NodeId, Opcode,
    SemanticOutcome, Value, ValueType, optimize_specialized_graph, verify_graph,
};

struct Fixture {
    graph: FunctionGraph,
    ty: TyId,
    control: NodeId,
    effect: NodeId,
    arg: NodeId,
}
impl Fixture {
    fn new(width: IntegerWidth, signed: bool) -> Self {
        let mut types = RecordedTypes::new();
        let bytes = u64::from(width.bits(64) / 8);
        let ty = types.intern(
            TypeShape::Int { width, signed },
            Some(SizeAlign {
                size: bytes,
                align: bytes,
            }),
        );
        let mut graph = FunctionGraph::new(
            "division",
            FunctionAbi {
                args: vec![PassMode::Direct],
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
                ValueType::Rust(ty),
            ]),
        );
        graph.start = start;
        let control = graph.add_node(
            Opcode::Projection,
            NodeId::NONE,
            &[start],
            0,
            ValueType::Control,
        );
        let effect = graph.add_node(
            Opcode::Projection,
            NodeId::NONE,
            &[start],
            1,
            ValueType::Effect,
        );
        let arg = graph.add_node(
            Opcode::Projection,
            NodeId::NONE,
            &[start],
            2,
            ValueType::Rust(ty),
        );
        Self {
            graph,
            ty,
            control,
            effect,
            arg,
        }
    }
    fn operation(&mut self, opcode: Opcode, numerator: NodeId, divisor: u64) -> NodeId {
        let divisor = self.graph.add_constant(u128::from(divisor), self.ty);
        self.graph.add_node(
            opcode,
            self.control,
            &[numerator, divisor],
            0,
            ValueType::Rust(self.ty),
        )
    }
    fn finish(&mut self, value: NodeId) {
        self.graph.add_node(
            Opcode::Return,
            self.control,
            &[self.effect, value],
            0,
            ValueType::Control,
        );
        verify_graph(&mut self.graph).unwrap();
    }
    fn optimize(&mut self) {
        optimize_specialized_graph(&mut self.graph).unwrap();
        verify_graph(&mut self.graph).unwrap();
    }
    fn contains_division(&self) -> bool {
        self.graph.ids().any(|n| {
            matches!(
                self.graph.op(n),
                Opcode::DivSigned | Opcode::DivUnsigned | Opcode::RemSigned | Opcode::RemUnsigned
            )
        })
    }
}
fn run(graph: &FunctionGraph, arg: Value) -> Result<SemanticOutcome, InterpretError> {
    Interpreter::new(graph).run_values(&[arg], &mut |name: &str, _: &[Value]| {
        Err(InterpretError::UnknownCallee(name.into()))
    })
}
fn expected(bits: u64, divisor: u64, width: u8, signed: bool, remainder: bool) -> SemanticOutcome {
    let mask = u64::MAX >> (64 - width);
    let result = if signed {
        let value = ((bits << (64 - width)) as i64) >> (64 - width);
        (if remainder {
            value % divisor as i64
        } else {
            value / divisor as i64
        }) as u64
    } else if remainder {
        (bits & mask) % divisor
    } else {
        (bits & mask) / divisor
    };
    SemanticOutcome::Return(Value::Scalar(result & mask))
}

#[test]
fn all_eight_bit_values_match_rust_division_and_remainder() {
    for signed in [false, true] {
        for remainder in [false, true] {
            for shift in 0..if signed { 7 } else { 8 } {
                let divisor = 1u64 << shift;
                let opcode = match (signed, remainder) {
                    (false, false) => Opcode::DivUnsigned,
                    (false, true) => Opcode::RemUnsigned,
                    (true, false) => Opcode::DivSigned,
                    (true, true) => Opcode::RemSigned,
                };
                let mut fixture = Fixture::new(IntegerWidth::W8, signed);
                let result = fixture.operation(opcode, fixture.arg, divisor);
                fixture.finish(result);
                let original = fixture.graph.clone();
                fixture.optimize();
                assert!(!fixture.contains_division());
                for bits in 0..=255 {
                    let answer = Ok(expected(bits, divisor, 8, signed, remainder));
                    assert_eq!(run(&original, Value::Scalar(bits)), answer);
                    assert_eq!(run(&fixture.graph, Value::Scalar(bits)), answer);
                }
            }
        }
    }
}

#[test]
fn larger_width_boundaries_and_generated_values_preserve_narrow_truncation() {
    for width in [IntegerWidth::W16, IntegerWidth::W32, IntegerWidth::W64] {
        let bits = width.bits(64);
        let mask = u64::MAX >> (64 - bits);
        let sign = 1u64 << (bits - 1);
        let mut values = vec![
            0,
            1,
            2,
            3,
            sign - 1,
            sign,
            sign + 1,
            mask - 2,
            mask - 1,
            mask,
        ];
        let mut seed = 0x7c3f912a500ff00du64;
        for _ in 0..64 {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
            values.push(seed & mask);
        }
        for signed in [false, true] {
            for remainder in [false, true] {
                for shift in [0, 1, bits - if signed { 2 } else { 1 }] {
                    let divisor = 1u64 << shift;
                    let opcode = match (signed, remainder) {
                        (false, false) => Opcode::DivUnsigned,
                        (false, true) => Opcode::RemUnsigned,
                        (true, false) => Opcode::DivSigned,
                        (true, true) => Opcode::RemSigned,
                    };
                    let mut fixture = Fixture::new(width, signed);
                    let result = fixture.operation(opcode, fixture.arg, divisor);
                    fixture.finish(result);
                    let original = fixture.graph.clone();
                    fixture.optimize();
                    assert!(!fixture.contains_division());
                    for &value in &values {
                        let answer = Ok(expected(value, divisor, bits, signed, remainder));
                        assert_eq!(run(&original, Value::Scalar(value)), answer);
                        assert_eq!(run(&fixture.graph, Value::Scalar(value)), answer);
                    }
                }
            }
        }
    }
}

#[test]
fn zero_and_negative_divisors_preserve_unused_traps() {
    for (opcode, divisor, numerator) in [
        (Opcode::DivUnsigned, 0, 17),
        (Opcode::RemUnsigned, 0, 17),
        (Opcode::DivSigned, 0, 17),
        (Opcode::RemSigned, 0, 17),
        (Opcode::DivSigned, u64::MAX, 1u64 << 63),
        (Opcode::RemSigned, u64::MAX, 1u64 << 63),
    ] {
        let signed = matches!(opcode, Opcode::DivSigned | Opcode::RemSigned);
        let mut fixture = Fixture::new(IntegerWidth::W64, signed);
        fixture.operation(opcode, fixture.arg, divisor);
        fixture.finish(fixture.arg);
        let before = run(&fixture.graph, Value::Scalar(numerator));
        assert!(
            matches!(before, Err(InterpretError::UndefinedBehaviour(ref message)) if message.ends_with(": division by zero or overflow"))
        );
        fixture.optimize();
        assert!(fixture.contains_division());
        assert!(
            matches!(run(&fixture.graph, Value::Scalar(numerator)), Err(InterpretError::UndefinedBehaviour(message)) if message.ends_with(": division by zero or overflow"))
        );
    }
}

#[test]
fn unknown_numerator_keeps_its_fixed_poison_observation_even_when_unused() {
    for opcode in [
        Opcode::DivUnsigned,
        Opcode::RemUnsigned,
        Opcode::DivSigned,
        Opcode::RemSigned,
    ] {
        let signed = matches!(opcode, Opcode::DivSigned | Opcode::RemSigned);
        let mut fixture = Fixture::new(IntegerWidth::W32, signed);
        let poison = fixture.graph.add_node(
            Opcode::Undefined,
            NodeId::NONE,
            &[],
            0,
            ValueType::Rust(fixture.ty),
        );
        fixture.operation(opcode, poison, 8);
        fixture.finish(fixture.arg);
        assert!(matches!(
            run(&fixture.graph, Value::Scalar(7)),
            Err(InterpretError::PoisonObserved(_))
        ));
        fixture.optimize();
        assert!(fixture.contains_division());
        let Err(InterpretError::PoisonObserved(observer)) = run(&fixture.graph, Value::Scalar(7))
        else {
            panic!("fixed poison observation lost");
        };
        assert_eq!(fixture.graph.op(observer), opcode);
    }
}

#[test]
fn entry_observes_poison_before_a_reduced_or_unused_division() {
    for unused in [false, true] {
        let mut fixture = Fixture::new(IntegerWidth::W64, false);
        let result = fixture.operation(Opcode::DivUnsigned, fixture.arg, 4);
        fixture.finish(if unused { fixture.arg } else { result });
        fixture.optimize();
        assert!(!fixture.contains_division());
        assert_eq!(
            run(&fixture.graph, Value::Poison),
            Err(InterpretError::PoisonObserved(fixture.graph.start))
        );
    }
}

#[test]
fn scalar_expression_proof_preserves_call_order_and_wrapping_arithmetic() {
    let mut fixture = Fixture::new(IntegerWidth::W16, true);
    let one = fixture.graph.add_constant(1, fixture.ty);
    let increment = fixture.graph.add_node(
        Opcode::Add,
        NodeId::NONE,
        &[fixture.arg, one],
        0,
        ValueType::Rust(fixture.ty),
    );
    let result = fixture.operation(Opcode::DivSigned, increment, 8);
    let target = fixture.graph.add_callee(CallTarget {
        symbol: "consume".into(),
        abi: FunctionAbi {
            args: vec![PassMode::Direct],
            ret: PassMode::Direct,
            c_variadic: false,
        },
    });
    let call = fixture.graph.add_node(
        Opcode::Call,
        fixture.control,
        &[fixture.effect, result],
        target,
        ValueType::Tuple(vec![
            ValueType::Control,
            ValueType::Effect,
            ValueType::Rust(fixture.ty),
        ]),
    );
    fixture.control = fixture.graph.add_node(
        Opcode::Projection,
        NodeId::NONE,
        &[call],
        0,
        ValueType::Control,
    );
    fixture.effect = fixture.graph.add_node(
        Opcode::Projection,
        NodeId::NONE,
        &[call],
        1,
        ValueType::Effect,
    );
    let result = fixture.graph.add_node(
        Opcode::Projection,
        NodeId::NONE,
        &[call],
        2,
        ValueType::Rust(fixture.ty),
    );
    fixture.finish(result);
    let original = fixture.graph.clone();
    fixture.optimize();
    assert!(!fixture.contains_division());
    let trace = |graph: &FunctionGraph, value| {
        let mut calls = vec![];
        let result = Interpreter::new(graph).run_values(
            &[Value::Scalar(value)],
            &mut |name: &str, args: &[Value]| {
                calls.push((name.to_owned(), args.to_vec()));
                Ok(args[0].clone())
            },
        );
        (result, calls)
    };
    for value in [0, 7, 32767, 32768, 65535] {
        assert_eq!(trace(&original, value), trace(&fixture.graph, value));
    }
}
