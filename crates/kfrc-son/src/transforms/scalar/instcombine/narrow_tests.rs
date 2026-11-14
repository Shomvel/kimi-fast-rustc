use super::*;
use crate::{
    ArithmeticFlags, GraphLevel, InterpretError, Interpreter, SemanticOutcome, Value,
    optimize_graph, verify_graph,
};
use kfrc_body::{
    FunctionAbi, IntegerWidth, PassMode, RecordedTypes, ScalarValidRange, ScalarValidity,
};

const BINOPS: [Opcode; 3] = [Opcode::BitAnd, Opcode::BitOr, Opcode::BitXor];

struct Fixture {
    graph: FunctionGraph,
    narrow: TyId,
    wide: TyId,
    args: [NodeId; 2],
    control: NodeId,
    effect: NodeId,
}

impl Fixture {
    fn new(narrow: TypeShape, wide: TypeShape, wide_args: [bool; 2]) -> Self {
        let mut types = RecordedTypes::new();
        let narrow = types.intern(narrow, None);
        let wide = types.intern(wide, None);
        let mut graph = FunctionGraph::new(
            "trunc_binop",
            FunctionAbi {
                args: vec![PassMode::Direct; 2],
                ret: PassMode::Direct,
                c_variadic: false,
            },
            types,
        );
        graph.level = GraphLevel::Specialized;
        let arg_types = wide_args.map(|is_wide| if is_wide { wide } else { narrow });
        graph.start = graph.add_node(
            Opcode::Start,
            NodeId::NONE,
            &[],
            0,
            ValueType::Tuple(vec![
                ValueType::Control,
                ValueType::Effect,
                ValueType::Rust(arg_types[0]),
                ValueType::Rust(arg_types[1]),
            ]),
        );
        let mut projection =
            |slot, ty| graph.add_node(Opcode::Projection, NodeId::NONE, &[graph.start], slot, ty);
        let control = projection(0, ValueType::Control);
        let effect = projection(1, ValueType::Effect);
        let args = [
            projection(2, ValueType::Rust(arg_types[0])),
            projection(3, ValueType::Rust(arg_types[1])),
        ];
        Self {
            graph,
            narrow,
            wide,
            args,
            control,
            effect,
        }
    }

    fn widen(&mut self, source: NodeId) -> NodeId {
        self.graph.add_node(
            Opcode::IntCast,
            NodeId::NONE,
            &[source],
            0,
            ValueType::Rust(self.wide),
        )
    }

    fn binop(&mut self, op: Opcode, left: NodeId, right: NodeId, flags: u32) -> NodeId {
        self.graph.add_node(
            op,
            NodeId::NONE,
            &[left, right],
            flags,
            ValueType::Rust(self.wide),
        )
    }

    fn truncate(&mut self, source: NodeId) -> NodeId {
        self.graph.add_node(
            Opcode::IntCast,
            NodeId::NONE,
            &[source],
            0,
            ValueType::Rust(self.narrow),
        )
    }

    fn finish(&mut self, values: &[NodeId]) {
        let result = if let [value] = values {
            *value
        } else {
            let fields = values
                .iter()
                .map(|&node| self.graph.rust_type(node).unwrap())
                .collect();
            let tuple = self.graph.oracle.intern(TypeShape::Tuple(fields), None);
            self.graph.add_node(
                Opcode::Aggregate,
                NodeId::NONE,
                values,
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

    fn rewrite(&mut self, node: NodeId) -> (FunctionGraph, NodeId) {
        let before = self.graph.clone();
        let uses = Uses::new(&self.graph);
        let replacement = combine(&mut self.graph, node, &uses).unwrap();
        self.graph.replace(node, replacement);
        self.graph.resolve_edges();
        verify_graph(&mut self.graph).unwrap();
        (before, replacement)
    }

    fn reject(&mut self, node: NodeId) {
        let uses = Uses::new(&self.graph);
        let before = self.graph.dump();
        assert!(combine(&mut self.graph, node, &uses).is_none());
        assert_eq!(
            before,
            self.graph.dump(),
            "rejection must not allocate nodes"
        );
    }
}

fn int(width: IntegerWidth, signed: bool) -> TypeShape {
    TypeShape::Int { width, signed }
}

fn mask(bits: u8) -> u64 {
    u64::MAX >> (64 - bits)
}

fn evaluate(op: Opcode, a: u64, b: u64, bits: u8) -> u64 {
    (match op {
        Opcode::Add => a.wrapping_add(b),
        Opcode::Sub => a.wrapping_sub(b),
        Opcode::Mul => a.wrapping_mul(b),
        Opcode::BitAnd => a & b,
        Opcode::BitOr => a | b,
        Opcode::BitXor => a ^ b,
        _ => unreachable!(),
    }) & mask(bits)
}

fn run(graph: &FunctionGraph, args: [u64; 2]) -> Result<SemanticOutcome, InterpretError> {
    Interpreter::new(graph).run_values(
        &args.map(Value::Scalar),
        &mut |_: &str, _: &[Value]| unreachable!(),
    )
}

fn values(bits: u8) -> Vec<u64> {
    let sign = 1 << (bits - 1);
    vec![0, 1, sign - 1, sign, mask(bits) - 1, mask(bits)]
}

fn live_count(graph: &FunctionGraph, opcode: Opcode) -> usize {
    let mut compact = graph.clone();
    compact.compact();
    compact
        .ids()
        .filter(|&node| compact.op(node) == opcode)
        .count()
}

#[test]
fn every_binop_width_and_signed_extension_preserves_low_bits() {
    for (narrow, wide) in [
        (IntegerWidth::W8, IntegerWidth::W16),
        (IntegerWidth::W8, IntegerWidth::W32),
        (IntegerWidth::W8, IntegerWidth::W64),
        (IntegerWidth::W8, IntegerWidth::Pointer),
        (IntegerWidth::W16, IntegerWidth::W32),
        (IntegerWidth::W16, IntegerWidth::W64),
        (IntegerWidth::W16, IntegerWidth::Pointer),
        (IntegerWidth::W32, IntegerWidth::W64),
        (IntegerWidth::W32, IntegerWidth::Pointer),
    ] {
        let bits = narrow.bits(64);
        for signed in [false, true] {
            for wide_signed in [false, true] {
                for op in BINOPS {
                    let mut f =
                        Fixture::new(int(narrow, signed), int(wide, wide_signed), [false; 2]);
                    let left = f.widen(f.args[0]);
                    let right = f.widen(f.args[1]);
                    let binop = f.binop(op, left, right, 0);
                    let truncation = f.truncate(binop);
                    f.finish(&[truncation]);
                    let (before, replacement) = f.rewrite(truncation);
                    assert_eq!(f.graph.op(replacement), op);
                    assert_eq!(f.graph.rust_type(replacement), Some(f.narrow));
                    assert_eq!(f.graph.payload(replacement), 0);
                    assert_eq!(live_count(&f.graph, Opcode::IntCast), 0);
                    let inputs: Vec<_> = if bits == 8 {
                        (0..=255).collect()
                    } else {
                        values(bits)
                    };
                    for a in inputs {
                        for b in [a.rotate_left(3) & mask(bits), 0, mask(bits)] {
                            let args = [a, b];
                            let expected = Ok(SemanticOutcome::Return(Value::Scalar(evaluate(
                                op, a, b, bits,
                            ))));
                            assert_eq!(run(&before, args), expected);
                            assert_eq!(
                                run(&f.graph, args),
                                expected,
                                "op={op:?}, narrow={narrow:?}, wide={wide:?}, signed={signed}, wide_signed={wide_signed}, args={args:?}"
                            );
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn constants_on_either_side_need_at_most_one_replacement_cast() {
    for signed in [false, true] {
        for wide_signed in [false, true] {
            for op in BINOPS {
                for constant_left in [false, true] {
                    for bits in [0x1_0000_0080u64, 0x1234_5678_1234, 0x8000_0000_0000_00ff] {
                        let mut f = Fixture::new(
                            int(IntegerWidth::W8, signed),
                            int(IntegerWidth::W64, wide_signed),
                            [true; 2],
                        );
                        let constant = f.graph.add_constant(u128::from(bits), f.wide);
                        let (a, b) = if constant_left {
                            (constant, f.args[0])
                        } else {
                            (f.args[0], constant)
                        };
                        let binop = f.binop(op, a, b, 0);
                        let truncation = f.truncate(binop);
                        f.finish(&[truncation]);
                        let (before, _) = f.rewrite(truncation);
                        assert_eq!(live_count(&f.graph, Opcode::IntCast), 1);
                        for value in [0, 1, 127, 128, 255, 256, 0x1234_5678_ffff_ff80, u64::MAX] {
                            assert_eq!(run(&before, [value, 0]), run(&f.graph, [value, 0]));
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn poison_from_either_operand_and_its_contract_reaches_the_same_observation() {
    for op in BINOPS {
        for poison_left in [false, true] {
            let mut f = Fixture::new(
                int(IntegerWidth::W8, false),
                int(IntegerWidth::W64, false),
                [false; 2],
            );
            let poison = f.graph.add_node(
                Opcode::Undefined,
                NodeId::NONE,
                &[],
                0,
                ValueType::Rust(f.narrow),
            );
            let left = f.widen(if poison_left { poison } else { f.args[0] });
            let right = f.widen(if poison_left { f.args[1] } else { poison });
            let binop = f.binop(op, left, right, 0);
            let truncation = f.truncate(binop);
            f.finish(&[truncation]);
            let (before, _) = f.rewrite(truncation);
            for args in [[0, 0], [255, 255]] {
                let expected = run(&before, args);
                assert!(matches!(expected, Err(InterpretError::PoisonObserved(_))));
                assert_eq!(expected, run(&f.graph, args));
            }
        }
    }
    let mut f = Fixture::new(
        int(IntegerWidth::W8, false),
        int(IntegerWidth::W64, false),
        [false; 2],
    );
    let one = f.graph.add_constant(1, f.narrow);
    let source = f.graph.add_node(
        Opcode::Add,
        NodeId::NONE,
        &[f.args[0], one],
        ArithmeticFlags::NUW,
        ValueType::Rust(f.narrow),
    );
    let left = f.widen(source);
    let right = f.graph.add_constant(19, f.wide);
    let binop = f.binop(Opcode::BitXor, left, right, 0);
    let truncation = f.truncate(binop);
    f.finish(&[truncation]);
    let (before, _) = f.rewrite(truncation);
    assert_eq!(f.graph.payload(source), ArithmeticFlags::NUW);
    for value in [0, 1, 126, 254, 255] {
        assert_eq!(run(&before, [value, 0]), run(&f.graph, [value, 0]));
    }
    assert!(matches!(
        run(&f.graph, [255, 0]),
        Err(InterpretError::PoisonObserved(_))
    ));
}

#[test]
fn promised_wide_operations_shared_results_and_two_new_casts_are_rejected() {
    for (op, flags) in [
        (Opcode::Add, ArithmeticFlags::NUW),
        (Opcode::Add, ArithmeticFlags::NSW),
        (Opcode::Sub, ArithmeticFlags::NUW),
        (Opcode::Sub, ArithmeticFlags::NSW),
        (Opcode::Mul, ArithmeticFlags::NUW),
        (Opcode::Mul, ArithmeticFlags::NSW),
        (Opcode::BitOr, ArithmeticFlags::DISJOINT),
    ] {
        let mut f = Fixture::new(
            int(IntegerWidth::W8, false),
            int(IntegerWidth::W64, false),
            [true; 2],
        );
        let constant = f.graph.add_constant(2, f.wide);
        let binop = f.binop(op, f.args[0], constant, flags);
        let truncation = f.truncate(binop);
        f.finish(&[truncation]);
        f.reject(truncation);
    }
    for op in BINOPS {
        for shared in [false, true] {
            let mut f = Fixture::new(
                int(IntegerWidth::W8, false),
                int(IntegerWidth::W64, false),
                [true; 2],
            );
            let right = if shared {
                f.graph.add_constant(17, f.wide)
            } else {
                f.args[1]
            };
            let binop = f.binop(op, f.args[0], right, 0);
            let truncation = f.truncate(binop);
            if shared {
                f.finish(&[truncation, binop]);
            } else {
                f.finish(&[truncation]);
            }
            f.reject(truncation);
        }
    }
}

#[test]
fn shared_extensions_remain_for_other_users_and_the_real_pipeline_narrows() {
    let mut f = Fixture::new(
        int(IntegerWidth::W8, true),
        int(IntegerWidth::W64, false),
        [false; 2],
    );
    let left = f.widen(f.args[0]);
    let right = f.widen(f.args[1]);
    let binop = f.binop(Opcode::BitXor, left, right, 0);
    let truncation = f.truncate(binop);
    f.finish(&[truncation, left]);
    let (before, _) = f.rewrite(truncation);
    assert_eq!(live_count(&f.graph, Opcode::IntCast), 1);
    for a in values(8) {
        for b in values(8) {
            assert_eq!(run(&before, [a, b]), run(&f.graph, [a, b]));
        }
    }
    let mut pipeline = before.clone();
    optimize_graph(&mut pipeline);
    verify_graph(&mut pipeline).unwrap();
    assert!(
        pipeline
            .ids()
            .any(|node| pipeline.op(node) == Opcode::BitXor
                && pipeline.rust_type(node) == Some(f.narrow))
    );
    for a in values(8) {
        for b in values(8) {
            assert_eq!(run(&before, [a, b]), run(&pipeline, [a, b]));
        }
    }
}

#[test]
fn nonordinary_types_controls_and_invalid_matcher_shapes_do_not_narrow() {
    for narrow in [
        TypeShape::Bool,
        TypeShape::Char,
        int(IntegerWidth::W64, false),
        int(IntegerWidth::W128, false),
    ] {
        let mut f = Fixture::new(narrow, int(IntegerWidth::W64, false), [true; 2]);
        let constant = f.graph.add_constant(17, f.wide);
        let binop = f.binop(Opcode::BitXor, f.args[0], constant, 0);
        let truncation = f.truncate(binop);
        f.reject(truncation);
    }
    for restriction in 0..7 {
        let mut f = Fixture::new(
            int(IntegerWidth::W8, false),
            int(IntegerWidth::W64, false),
            [false; 2],
        );
        let extension = f.widen(f.args[0]);
        let constant = f.graph.add_constant(17, f.wide);
        let binop = f.binop(Opcode::BitXor, extension, constant, 0);
        let truncation = f.truncate(binop);
        match restriction {
            0 => f.graph.oracle.record_scalar_validity(
                f.narrow,
                ScalarValidity::Scalar(ScalarValidRange::new(8, 1, 255).unwrap()),
            ),
            1 => f.graph.oracle.record_scalar_validity(
                f.wide,
                ScalarValidity::Scalar(ScalarValidRange::new(64, 1, u64::MAX as u128).unwrap()),
            ),
            2 => f.graph.set_control(truncation, f.control),
            3 => f.graph.set_control(binop, f.control),
            4 => f.graph.set_control(extension, f.control),
            5 => f.graph.set_payload(truncation, 1),
            _ => f.graph.set_payload(extension, 1),
        }
        // Nonzero IntCast payloads are invalid Specialized IR. Check their
        // matcher boundary directly without interpreting an invalid graph.
        f.reject(truncation);
    }
    for shape in [TypeShape::Bool, TypeShape::Char] {
        let mut f = Fixture::new(
            int(IntegerWidth::W8, false),
            int(IntegerWidth::W64, false),
            [false; 2],
        );
        let source_ty = f.graph.oracle.intern(shape, None);
        let source = f.graph.add_node(
            Opcode::Undefined,
            NodeId::NONE,
            &[],
            0,
            ValueType::Rust(source_ty),
        );
        let extension = f.widen(source);
        let constant = f.graph.add_constant(17, f.wide);
        let binop = f.binop(Opcode::BitXor, extension, constant, 0);
        let truncation = f.truncate(binop);
        f.reject(truncation);
    }
}

#[test]
fn a_forwarded_producer_waits_for_a_new_use_snapshot() {
    let mut f = Fixture::new(
        int(IntegerWidth::W8, false),
        int(IntegerWidth::W64, false),
        [true; 2],
    );
    let constant = f.graph.add_constant(0x1234_5678_1234, f.wide);
    let old = f.binop(Opcode::BitXor, f.args[0], constant, 0);
    let truncation = f.truncate(old);
    f.finish(&[truncation]);
    let uses = Uses::new(&f.graph);
    let new = f.binop(Opcode::BitOr, f.args[0], constant, 0);
    f.graph.replace(old, new);
    assert!(combine(&mut f.graph, truncation, &uses).is_none());
    f.graph.resolve_edges();
    let uses = Uses::new(&f.graph);
    assert!(combine(&mut f.graph, truncation, &uses).is_some());
}

#[test]
fn arithmetic_narrowing_waits_for_profitable_target_lowering() {
    for op in [Opcode::Add, Opcode::Sub, Opcode::Mul] {
        for extended in [false, true] {
            let mut f = Fixture::new(
                int(IntegerWidth::W16, true),
                int(IntegerWidth::W64, false),
                [!extended; 2],
            );
            let left = if extended {
                f.widen(f.args[0])
            } else {
                f.args[0]
            };
            let right = f.graph.add_constant(0x1234_5678_1234, f.wide);
            let binop = f.binop(op, left, right, 0);
            let truncation = f.truncate(binop);
            f.finish(&[truncation]);
            f.reject(truncation);
        }
    }
}

#[test]
fn mixed_width_inputs_do_not_add_a_canonicalization_without_an_immediate_saving() {
    for signed in [false, true] {
        for op in BINOPS {
            for extension_left in [false, true] {
                for poison in [false, true] {
                    let mut f = Fixture::new(
                        int(IntegerWidth::W8, signed),
                        int(IntegerWidth::W64, false),
                        [false, true],
                    );
                    let extension = f.widen(f.args[0]);
                    let wide = if poison {
                        f.graph.add_node(
                            Opcode::Undefined,
                            NodeId::NONE,
                            &[],
                            0,
                            ValueType::Rust(f.wide),
                        )
                    } else {
                        f.args[1]
                    };
                    let (left, right) = if extension_left {
                        (extension, wide)
                    } else {
                        (wide, extension)
                    };
                    let binop = f.binop(op, left, right, 0);
                    let truncation = f.truncate(binop);
                    f.finish(&[truncation]);
                    f.reject(truncation);
                }
            }
        }
    }
}

#[test]
fn encodable_immediates_do_not_pay_for_an_extra_input_cast() {
    for op in BINOPS {
        for signed in [false, true] {
            for width in [IntegerWidth::W16, IntegerWidth::W32, IntegerWidth::W64] {
                for bits in [0u64, 1, 0x3f, 0x80, 0x1234, 0xffff_ffff_ffff_ff80] {
                    let mut f =
                        Fixture::new(int(IntegerWidth::W8, signed), int(width, false), [true; 2]);
                    let constant = f
                        .graph
                        .add_constant(u128::from(bits & mask(width.bits(64))), f.wide);
                    let binop = f.binop(op, f.args[0], constant, 0);
                    let truncation = f.truncate(binop);
                    f.finish(&[truncation]);
                    f.reject(truncation);
                }
            }
        }
    }
    for op in BINOPS {
        for bits in [0x1234_abcd_8000_0000u64, 0x1234_abcd_ffff_ffff] {
            let mut f = Fixture::new(
                int(IntegerWidth::W32, false),
                int(IntegerWidth::W64, false),
                [true; 2],
            );
            let constant = f.graph.add_constant(u128::from(bits), f.wide);
            let binop = f.binop(op, f.args[0], constant, 0);
            let truncation = f.truncate(binop);
            f.finish(&[truncation]);
            f.reject(truncation);
        }
    }
}
