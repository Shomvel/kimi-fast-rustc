use kfrc_body::{
    FunctionAbi, IntegerWidth, PassMode, RecordedTypes, ScalarValidRange, ScalarValidity, TyId,
    TypeShape,
};
use kfrc_son::{
    ArithmeticFlags, ComparePredicate, FunctionGraph, GraphLevel, Interpreter, NodeId, Opcode,
    SemanticOutcome, Value, ValueType, optimize_graph, verify_graph,
};

struct Fixture {
    graph: FunctionGraph,
    source_ty: TyId,
    target_ty: TyId,
    boolean: TyId,
    source: NodeId,
    extension: NodeId,
    control: NodeId,
    effect: NodeId,
}

impl Fixture {
    fn new(source: TypeShape, target: TypeShape) -> Self {
        let mut types = RecordedTypes::new();
        let source_ty = types.intern(source, None);
        let target_ty = types.intern(target, None);
        let boolean = types.intern(TypeShape::Bool, None);
        let mut graph = FunctionGraph::new(
            "ext_compare",
            FunctionAbi {
                args: vec![PassMode::Direct],
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
                ValueType::Rust(source_ty),
            ]),
        );
        let control = graph.add_node(
            Opcode::Projection,
            NodeId::NONE,
            &[graph.start],
            0,
            ValueType::Control,
        );
        let effect = graph.add_node(
            Opcode::Projection,
            NodeId::NONE,
            &[graph.start],
            1,
            ValueType::Effect,
        );
        let source = graph.add_node(
            Opcode::Projection,
            NodeId::NONE,
            &[graph.start],
            2,
            ValueType::Rust(source_ty),
        );
        let extension = graph.add_node(
            Opcode::IntCast,
            NodeId::NONE,
            &[source],
            0,
            ValueType::Rust(target_ty),
        );
        Self {
            graph,
            source_ty,
            target_ty,
            boolean,
            source,
            extension,
            control,
            effect,
        }
    }

    fn compare(&mut self, predicate: ComparePredicate, bits: u64, swapped: bool) -> NodeId {
        let constant = self.graph.add_constant(u128::from(bits), self.target_ty);
        let operands = if swapped {
            [constant, self.extension]
        } else {
            [self.extension, constant]
        };
        self.graph.add_node(
            Opcode::Compare,
            NodeId::NONE,
            &operands,
            predicate as u32,
            ValueType::Rust(self.boolean),
        )
    }

    fn finish(&mut self, results: &[NodeId]) {
        let result = if results.len() == 1 {
            results[0]
        } else {
            let fields = results
                .iter()
                .map(|&node| self.graph.rust_type(node).unwrap())
                .collect();
            let tuple = self.graph.oracle.intern(TypeShape::Tuple(fields), None);
            self.graph.add_node(
                Opcode::Aggregate,
                NodeId::NONE,
                results,
                0,
                ValueType::Rust(tuple),
            )
        };
        self.graph.add_node(
            Opcode::Return,
            self.control,
            &[self.effect, result],
            0,
            ValueType::Control,
        );
        verify_graph(&mut self.graph).unwrap();
    }

    fn optimize(&mut self) -> FunctionGraph {
        let before = self.graph.clone();
        optimize_graph(&mut self.graph);
        verify_graph(&mut self.graph).unwrap();
        before
    }
}

fn int(width: IntegerWidth, signed: bool) -> TypeShape {
    TypeShape::Int { width, signed }
}

fn mask(bits: u8) -> u64 {
    u64::MAX >> (64 - bits)
}

fn extend(value: u64, bits: u8, signed: bool, target_bits: u8) -> u64 {
    let value = if signed {
        let shift = 64 - bits;
        (((value << shift) as i64) >> shift) as u64
    } else {
        value & mask(bits)
    };
    value & mask(target_bits)
}

fn run(
    graph: &FunctionGraph,
    argument: Value,
) -> Result<SemanticOutcome, kfrc_son::InterpretError> {
    Interpreter::new(graph).run_values(&[argument], &mut |_: &str, _: &[Value]| unreachable!())
}

fn count(graph: &FunctionGraph, opcode: Opcode) -> usize {
    graph.ids().filter(|&node| graph.op(node) == opcode).count()
}

fn assert_narrowed(f: &Fixture) {
    let comparisons: Vec<_> = f
        .graph
        .ids()
        .filter(|&node| f.graph.op(node) == Opcode::Compare)
        .collect();
    assert!(!comparisons.is_empty());
    for node in comparisons {
        assert_eq!(f.graph.rust_type(f.graph.input(node, 0)), Some(f.source_ty));
        assert_eq!(f.graph.rust_type(f.graph.input(node, 1)), Some(f.source_ty));
    }
}

#[test]
fn every_i8_value_predicate_and_representable_constant_is_equivalent() {
    for signed in [false, true] {
        for target_signed in [false, true] {
            for swapped in [false, true] {
                for predicate in ComparePredicate::ALL {
                    let mut f = Fixture::new(
                        int(IntegerWidth::W8, signed),
                        int(IntegerWidth::W16, target_signed),
                    );
                    // One returned tuple covers every constant. This exercises
                    // the real pass while avoiding a fresh graph for every X.
                    let comparisons: Vec<_> = (0..=255)
                        .map(|constant| {
                            f.compare(predicate, extend(constant, 8, signed, 16), swapped)
                        })
                        .collect();
                    f.finish(&comparisons);
                    let before = f.optimize();
                    assert_eq!(count(&f.graph, Opcode::IntCast), 0);
                    assert_narrowed(&f);
                    for value in 0..=255 {
                        assert_eq!(
                            run(&before, Value::Scalar(value)),
                            run(&f.graph, Value::Scalar(value)),
                            "signed={signed}, target_signed={target_signed}, swapped={swapped}, predicate={predicate:?}, value={value}"
                        );
                    }
                    assert!(run(&before, Value::Poison).is_err());
                    assert!(run(&f.graph, Value::Poison).is_err());
                }
            }
        }
    }
}

#[test]
fn wider_sources_targets_and_shared_cast_uses_keep_their_semantics() {
    for source in [IntegerWidth::W8, IntegerWidth::W16, IntegerWidth::W32] {
        for target in [
            IntegerWidth::W16,
            IntegerWidth::W32,
            IntegerWidth::W64,
            IntegerWidth::Pointer,
        ] {
            if source.bits(64) >= target.bits(64) {
                continue;
            }
            for signed in [false, true] {
                for target_signed in [false, true] {
                    let mut f = Fixture::new(int(source, signed), int(target, target_signed));
                    let sign = 1u64 << (source.bits(64) - 1);
                    let values = [
                        0,
                        1,
                        sign - 1,
                        sign,
                        mask(source.bits(64)) - 1,
                        mask(source.bits(64)),
                    ];
                    let mut results = Vec::new();
                    for predicate in ComparePredicate::ALL {
                        for swapped in [false, true] {
                            for value in values {
                                results.push(f.compare(
                                    predicate,
                                    extend(value, source.bits(64), signed, target.bits(64)),
                                    swapped,
                                ));
                            }
                        }
                    }
                    results.push(f.extension);
                    f.finish(&results);
                    let before = f.optimize();
                    assert_eq!(
                        count(&f.graph, Opcode::IntCast),
                        1,
                        "the other cast use remains"
                    );
                    assert_narrowed(&f);
                    for value in values {
                        assert_eq!(
                            run(&before, Value::Scalar(value)),
                            run(&f.graph, Value::Scalar(value))
                        );
                    }
                    assert!(run(&before, Value::Poison).is_err());
                    assert!(run(&f.graph, Value::Poison).is_err());
                }
            }
        }
    }
}

#[test]
fn undefined_and_overflowing_sources_are_still_observed() {
    for signed in [false, true] {
        for representable in [false, true] {
            let mut f = Fixture::new(int(IntegerWidth::W8, signed), int(IntegerWidth::W64, false));
            let undefined = f.graph.add_node(
                Opcode::Undefined,
                NodeId::NONE,
                &[],
                0,
                ValueType::Rust(f.source_ty),
            );
            f.graph.set_input(f.extension, 0, undefined);
            let comparison = f.compare(
                ComparePredicate::UnsignedLess,
                if representable { 64 } else { 256 },
                false,
            );
            f.finish(&[comparison]);
            let before = f.optimize();
            assert_eq!(
                count(&f.graph, Opcode::IntCast),
                usize::from(!representable)
            );
            assert_eq!(count(&f.graph, Opcode::Compare), 1);
            assert!(run(&before, Value::Scalar(0)).is_err());
            assert!(run(&f.graph, Value::Scalar(0)).is_err());
        }
    }
    let mut f = Fixture::new(int(IntegerWidth::W8, false), int(IntegerWidth::W64, false));
    let one = f.graph.add_constant(1, f.source_ty);
    let addition = f.graph.add_node(
        Opcode::Add,
        NodeId::NONE,
        &[f.source, one],
        ArithmeticFlags::NUW,
        ValueType::Rust(f.source_ty),
    );
    f.graph.set_input(f.extension, 0, addition);
    let comparison = f.compare(ComparePredicate::UnsignedLess, 64, false);
    f.finish(&[comparison]);
    let before = f.optimize();
    assert_eq!(count(&f.graph, Opcode::IntCast), 0);
    assert!(f.graph.ids().any(
        |node| f.graph.op(node) == Opcode::Add && f.graph.payload(node) == ArithmeticFlags::NUW
    ));
    for value in 0..=254 {
        assert_eq!(
            run(&before, Value::Scalar(value)),
            run(&f.graph, Value::Scalar(value))
        );
    }
    assert!(run(&before, Value::Scalar(255)).is_err());
    assert!(run(&f.graph, Value::Scalar(255)).is_err());
}

#[test]
fn supported_non_extensions_and_restricted_casts_are_not_narrowed() {
    // W128 and nonzero IntCast payloads are not valid Specialized IR. Their
    // defensive matcher rejection is covered privately, without passing an
    // invalid graph through the semantic optimizer or interpreter.
    let pairs: [(TypeShape, TypeShape, &[u64]); 6] = [
        (TypeShape::Bool, int(IntegerWidth::W64, false), &[0, 1]),
        (
            TypeShape::Char,
            int(IntegerWidth::W64, false),
            &[0, 1, 127, 0xd7ff, 0xe000, 0x10ffff],
        ),
        (
            int(IntegerWidth::W8, false),
            TypeShape::Char,
            &[0, 1, 127, 128, 255],
        ),
        (
            int(IntegerWidth::W8, false),
            TypeShape::Bool,
            &[0, 1, 127, 128, 255],
        ),
        (
            int(IntegerWidth::W8, false),
            int(IntegerWidth::W8, true),
            &[0, 1, 127, 128, 255],
        ),
        (
            int(IntegerWidth::W16, false),
            int(IntegerWidth::W8, false),
            &[0, 1, 127, 128, 255, 256, 65535],
        ),
    ];
    for (source, target, values) in pairs {
        let mut f = Fixture::new(source, target);
        let comparison = f.compare(ComparePredicate::Equal, 1, false);
        f.finish(&[comparison]);
        let before = f.optimize();
        assert_eq!(count(&f.graph, Opcode::IntCast), 1);
        for &value in values {
            assert_eq!(
                run(&before, Value::Scalar(value)),
                run(&f.graph, Value::Scalar(value))
            );
        }
        assert!(run(&before, Value::Poison).is_err());
        assert!(run(&f.graph, Value::Poison).is_err());
    }
    for restriction in 0..3 {
        let mut f = Fixture::new(int(IntegerWidth::W8, false), int(IntegerWidth::W64, false));
        match restriction {
            0 => f.graph.oracle.record_scalar_validity(
                f.source_ty,
                ScalarValidity::Scalar(ScalarValidRange::new(8, 1, 255).unwrap()),
            ),
            1 => f.graph.oracle.record_scalar_validity(
                f.target_ty,
                ScalarValidity::Scalar(ScalarValidRange::new(64, 1, u64::MAX as u128).unwrap()),
            ),
            _ => f.graph.set_control(f.extension, f.control),
        }
        let comparison = f.compare(ComparePredicate::Equal, 17, false);
        f.finish(&[comparison]);
        let before = f.optimize();
        assert_eq!(
            count(&f.graph, Opcode::IntCast),
            1,
            "restriction={restriction}"
        );
        for value in [1, 17, 255] {
            assert_eq!(
                run(&before, Value::Scalar(value)),
                run(&f.graph, Value::Scalar(value))
            );
        }
        assert!(run(&before, Value::Poison).is_err());
        assert!(run(&f.graph, Value::Poison).is_err());
    }
}

#[test]
fn unrepresentable_constants_stay_wide_and_the_rewrite_converges() {
    for (signed, constants) in [
        (false, [256, u64::MAX]),
        (true, [128, 0xffff_ffff_ffff_ff7f]),
    ] {
        let mut f = Fixture::new(int(IntegerWidth::W8, signed), int(IntegerWidth::W64, false));
        let mut results = Vec::new();
        for predicate in ComparePredicate::ALL {
            for constant in constants {
                results.push(f.compare(predicate, constant, false));
            }
        }
        f.finish(&results);
        let before = f.optimize();
        for value in 0..=255 {
            assert_eq!(
                run(&before, Value::Scalar(value)),
                run(&f.graph, Value::Scalar(value))
            );
        }
    }
    let mut f = Fixture::new(int(IntegerWidth::W8, false), int(IntegerWidth::W64, false));
    let comparison = f.compare(ComparePredicate::SignedLess, 64, true);
    f.finish(&[comparison]);
    f.optimize();
    let once = f.graph.dump();
    optimize_graph(&mut f.graph);
    verify_graph(&mut f.graph).unwrap();
    assert_eq!(once, f.graph.dump());
}
