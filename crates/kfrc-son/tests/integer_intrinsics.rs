//! Integer intrinsic carriers survive R/S/L and preserve source contracts.
use kfrc_body::*;
use kfrc_son::specialize::{HoleAnswer, RustDefinition, SpecializationOracle};
use kfrc_son::{
    FunctionGraph, GraphLevel, Hole, IntegerIntrinsicPayload as P, InterpretError, Interpreter,
    NodeId, Opcode, Outcome, RustSignature, SemanticOutcome, Value, ValueType, build_graph,
    lower_layout, optimize_specialized_graph, verify_graph,
};

fn graph(op: Opcode, payload: u32, width: IntegerWidth, signed: bool) -> FunctionGraph {
    let mut types = RecordedTypes::new();
    let word = types.intern(
        TypeShape::Int { width, signed },
        Some(SizeAlign {
            size: u64::from(width.bits(64) / 8),
            align: u64::from(width.bits(64) / 8),
        }),
    );
    let u32_ = types.intern(
        TypeShape::Int {
            width: IntegerWidth::W32,
            signed: false,
        },
        Some(SizeAlign { size: 4, align: 4 }),
    );
    let arity = op.integer_intrinsic_arity().unwrap();
    let args: Vec<_> = (0..arity)
        .map(|index| {
            if op == Opcode::FunnelShift && index == 2 {
                u32_
            } else {
                word
            }
        })
        .collect();
    let result_ty = if op == Opcode::BitCount { u32_ } else { word };
    let mut g = FunctionGraph::new(
        "intrinsic",
        FunctionAbi {
            args: vec![PassMode::Direct; arity],
            ret: PassMode::Direct,
            c_variadic: false,
        },
        types,
    );
    g.level = GraphLevel::Specialized;
    let mut outputs = vec![ValueType::Control, ValueType::Effect];
    outputs.extend(args.iter().copied().map(ValueType::Rust));
    let start = g.add_node(
        Opcode::Start,
        NodeId::NONE,
        &[],
        0,
        ValueType::Tuple(outputs),
    );
    g.start = start;
    let control = g.add_node(
        Opcode::Projection,
        NodeId::NONE,
        &[start],
        0,
        ValueType::Control,
    );
    let effect = g.add_node(
        Opcode::Projection,
        NodeId::NONE,
        &[start],
        1,
        ValueType::Effect,
    );
    let inputs: Vec<_> = args
        .iter()
        .enumerate()
        .map(|(i, &ty)| {
            g.add_node(
                Opcode::Projection,
                NodeId::NONE,
                &[start],
                i as u32 + 2,
                ValueType::Rust(ty),
            )
        })
        .collect();
    let value = g.add_node(
        op,
        NodeId::NONE,
        &inputs,
        payload,
        ValueType::Rust(result_ty),
    );
    g.add_node(
        Opcode::Return,
        control,
        &[effect, value],
        0,
        ValueType::Control,
    );
    g
}

fn run(g: &FunctionGraph, args: &[u64]) -> Result<u64, &'static str> {
    let result = if g.level == GraphLevel::Layout {
        Interpreter::new(g)
            .run(args, &mut |_: &str, _: &[u64]| unreachable!())
            .map(|r| match r {
                Outcome::Return(v) => v,
                _ => panic!("unexpected return"),
            })
    } else {
        Interpreter::new(g)
            .run_values(
                &args.iter().copied().map(Value::Scalar).collect::<Vec<_>>(),
                &mut |_: &str, _: &[Value]| unreachable!(),
            )
            .map(|r| match r {
                SemanticOutcome::Return(Value::Scalar(v)) => v,
                _ => panic!("unexpected return"),
            })
    };
    result.map_err(|e| match e {
        InterpretError::PoisonObserved(_) => "poison",
        other => panic!("unexpected error: {other:?}"),
    })
}

fn stages(mut g: FunctionGraph) -> Vec<FunctionGraph> {
    verify_graph(&mut g).unwrap();
    let original = g.clone();
    optimize_specialized_graph(&mut g).unwrap();
    let optimized = g.clone();
    lower_layout(&mut g).unwrap();
    verify_graph(&mut g).unwrap();
    vec![original, optimized, g]
}

#[test]
fn all_byte_unary_values_and_boundary_funnels_preserve_s_and_l_behavior() {
    for (op, payload) in [
        (Opcode::BitCount, 0),
        (Opcode::BitCount, 1),
        (Opcode::BitCount, 2),
        (Opcode::BitCount, 5),
        (Opcode::BitCount, 6),
        (Opcode::ByteSwap, 0),
        (Opcode::BitReverse, 0),
    ] {
        let stages = stages(graph(op, payload, IntegerWidth::W8, false));
        for a in 0u8..=255 {
            let expected = if op == Opcode::BitCount && payload & P::ZERO_POISON != 0 && a == 0 {
                Err("poison")
            } else {
                Ok(match op {
                    Opcode::BitCount => u64::from(match payload & 3 {
                        0 => a.count_ones(),
                        1 => a.leading_zeros(),
                        _ => a.trailing_zeros(),
                    }),
                    Opcode::ByteSwap => u64::from(a),
                    _ => u64::from(a.reverse_bits()),
                })
            };
            for g in &stages {
                assert_eq!(
                    run(g, &[u64::from(a)]),
                    expected,
                    "{op:?} payload={payload} a={a}"
                );
            }
        }
    }
    for payload in 0..4 {
        let stages = stages(graph(Opcode::FunnelShift, payload, IntegerWidth::W8, false));
        for a in 0u16..=255 {
            let b = 255 - a;
            for amount in [0u64, 1, 7, 8, 9, 31, 255, 256, u64::from(u32::MAX)] {
                let shift = amount % 8;
                let joined = (u32::from(a) << 8) | u32::from(b);
                let expected = if payload & P::AMOUNT_IN_RANGE != 0 && amount >= 8 {
                    Err("poison")
                } else {
                    Ok(u64::from(if payload & P::FUNNEL_RIGHT != 0 {
                        (joined >> shift) & 255
                    } else {
                        ((joined << shift) >> 8) & 255
                    }))
                };
                for g in &stages {
                    assert_eq!(run(g, &[u64::from(a), u64::from(b), amount]), expected);
                }
            }
        }
    }
}

#[test]
fn saturating_boundary_pairs_preserve_signedness_across_layout() {
    for signed in [false, true] {
        for op in [Opcode::SaturatingAdd, Opcode::SaturatingSub] {
            let stages = stages(graph(op, 0, IntegerWidth::W8, signed));
            for a in 0u16..=255 {
                for b in [0u16, 1, 2, 127, 128, 254, 255] {
                    let expected = if signed {
                        let (a, b) = (a as i8, b as i8);
                        (if op == Opcode::SaturatingAdd {
                            a.saturating_add(b)
                        } else {
                            a.saturating_sub(b)
                        }) as u8
                    } else {
                        let (a, b) = (a as u8, b as u8);
                        if op == Opcode::SaturatingAdd {
                            a.saturating_add(b)
                        } else {
                            a.saturating_sub(b)
                        }
                    };
                    for g in &stages {
                        assert_eq!(
                            run(g, &[u64::from(a), u64::from(b)]),
                            Ok(u64::from(expected))
                        );
                    }
                }
            }
        }
    }
}

fn intrinsic_node(g: &FunctionGraph) -> NodeId {
    g.ids()
        .find(|&n| g.op(n).integer_intrinsic_arity().is_some())
        .unwrap()
}

#[test]
fn constants_and_identities_do_not_erase_intrinsic_contract_poison() {
    for (op, payload, values) in [
        (Opcode::BitCount, 1, vec![0]),
        (Opcode::BitCount, 5, vec![0]),
        (Opcode::ByteSwap, 0, vec![0x19]),
        (Opcode::BitReverse, 0, vec![0x19]),
        (Opcode::FunnelShift, 0, vec![0x12, 0x34, 8]),
        (Opcode::FunnelShift, 2, vec![0x12, 0x34, 256]),
        (Opcode::SaturatingAdd, 0, vec![250, 20]),
        (Opcode::SaturatingSub, 0, vec![1, 20]),
    ] {
        let mut g = graph(op, payload, IntegerWidth::W8, false);
        let node = intrinsic_node(&g);
        let args = g.inputs(node).to_vec();
        for (slot, (&arg, &bits)) in args.iter().zip(&values).enumerate() {
            let ty = g.rust_type(arg).unwrap();
            let c = g.add_constant(u128::from(bits), ty);
            g.set_input(node, slot, c);
        }
        verify_graph(&mut g).unwrap();
        let expected = run(&g, &values);
        optimize_specialized_graph(&mut g).unwrap();
        assert_eq!(run(&g, &values), expected);
        assert_eq!(g.ids().any(|n| g.op(n) == op), expected.is_err());
    }
    // Funnel amount zero cannot erase poison from the other data half.
    let mut g = graph(Opcode::FunnelShift, 0, IntegerWidth::W8, false);
    let n = intrinsic_node(&g);
    let inputs = g.inputs(n).to_vec();
    let poison = g.add_node(
        Opcode::Undefined,
        NodeId::NONE,
        &[],
        0,
        g.value_type(inputs[1]).clone(),
    );
    let zero = g.add_constant(0, g.rust_type(inputs[2]).unwrap());
    g.set_input(n, 1, poison);
    g.set_input(n, 2, zero);
    for g in stages(g) {
        assert_eq!(run(&g, &[17, 0, 0]), Err("poison"));
    }
}

#[test]
fn verifier_rejects_bad_intrinsic_payload_width_and_types() {
    for (op, payload) in [
        (Opcode::BitCount, 3),
        (Opcode::BitCount, 4),
        (Opcode::BitCount, 8),
        (Opcode::FunnelShift, 4),
        (Opcode::ByteSwap, 1),
        (Opcode::SaturatingAdd, 1),
    ] {
        assert!(verify_graph(&mut graph(op, payload, IntegerWidth::W8, false)).is_err());
    }
    assert!(verify_graph(&mut graph(Opcode::BitReverse, 0, IntegerWidth::W128, false)).is_err());
    let mut g = graph(Opcode::BitCount, 0, IntegerWidth::W8, false);
    let n = intrinsic_node(&g);
    g.set_value_type(n, g.value_type(g.input(n, 0)).clone());
    assert!(verify_graph(&mut g).is_err());
    let mut g = graph(Opcode::FunnelShift, 0, IntegerWidth::W8, false);
    let n = intrinsic_node(&g);
    let signed = g.oracle.intern(
        TypeShape::Int {
            width: IntegerWidth::W32,
            signed: true,
        },
        Some(SizeAlign { size: 4, align: 4 }),
    );
    let c = g.add_constant(1, signed);
    g.set_input(n, 2, c);
    assert!(verify_graph(&mut g).is_err());
    let mut g = graph(Opcode::BitCount, 0, IntegerWidth::W8, false);
    let n = intrinsic_node(&g);
    g.set_inputs(n, &[]);
    assert!(verify_graph(&mut g).is_err());
}

#[test]
fn reversals_and_zero_rotations_preserve_poison_and_gvn_keeps_contracts() {
    for op in [Opcode::ByteSwap, Opcode::BitReverse] {
        let mut g = graph(op, 0, IntegerWidth::W16, false);
        let inner = intrinsic_node(&g);
        let outer = g.add_node(op, NodeId::NONE, &[inner], 0, g.value_type(inner).clone());
        let ret = g.ids().find(|&n| g.op(n) == Opcode::Return).unwrap();
        g.set_input(ret, 1, outer);
        for g in stages(g) {
            for value in [0u64, 1, 255, 256, 32768, 65535] {
                assert_eq!(run(&g, &[value]), Ok(value));
            }
        }
    }
    for amount in [0u64, 8, 256] {
        let mut g = graph(Opcode::FunnelShift, 0, IntegerWidth::W8, false);
        let n = intrinsic_node(&g);
        let input = g.input(n, 0);
        let c = g.add_constant(u128::from(amount), g.rust_type(g.input(n, 2)).unwrap());
        g.set_inputs(n, &[input, input, c]);
        optimize_specialized_graph(&mut g).unwrap();
        assert!(!g.ids().any(|n| g.op(n) == Opcode::FunnelShift));
        assert_eq!(run(&g, &[173, 0, 0]), Ok(173));
    }
    let mut g = graph(Opcode::BitCount, P::TRAILING_ZEROS, IntegerWidth::W8, false);
    let plain = intrinsic_node(&g);
    let flagged = g.add_node(
        Opcode::BitCount,
        NodeId::NONE,
        &[g.input(plain, 0)],
        P::TRAILING_ZEROS | P::ZERO_POISON,
        g.value_type(plain).clone(),
    );
    let sum = g.add_node(
        Opcode::Add,
        NodeId::NONE,
        &[plain, flagged],
        0,
        g.value_type(plain).clone(),
    );
    let ret = g.ids().find(|&n| g.op(n) == Opcode::Return).unwrap();
    g.set_input(ret, 1, sum);
    optimize_specialized_graph(&mut g).unwrap();
    assert_eq!(g.ids().filter(|&n| g.op(n) == Opcode::BitCount).count(), 2);
    assert_eq!(run(&g, &[0]), Err("poison"));
    assert_eq!(run(&g, &[4]), Ok(4));
}

#[test]
fn count_range_requires_defined_input_and_nonzero_proof() {
    use kfrc_son::analysis::ranges::{RangeFacts, ValueRange};
    for (payload, constant, expected) in [
        (
            P::LEADING_ZEROS,
            None,
            Some(ValueRange {
                bits: 32,
                min: 0,
                max: 8,
            }),
        ),
        (P::LEADING_ZEROS | P::ZERO_POISON, None, None),
        (
            P::LEADING_ZEROS | P::ZERO_POISON,
            Some(1),
            Some(ValueRange {
                bits: 32,
                min: 0,
                max: 7,
            }),
        ),
        (P::LEADING_ZEROS | P::ZERO_POISON, Some(0), None),
    ] {
        let mut g = graph(Opcode::BitCount, payload, IntegerWidth::W8, false);
        let n = intrinsic_node(&g);
        if let Some(value) = constant {
            let c = g.add_constant(value, g.rust_type(g.input(n, 0)).unwrap());
            g.set_input(n, 0, c);
        }
        assert_eq!(RangeFacts::new(&g).range_of(n), expected);
    }
    let mut g = graph(Opcode::BitCount, P::TRAILING_ZEROS, IntegerWidth::W8, false);
    let n = intrinsic_node(&g);
    let poison = g.add_node(
        Opcode::Undefined,
        NodeId::NONE,
        &[],
        0,
        g.value_type(g.input(n, 0)).clone(),
    );
    g.set_input(n, 0, poison);
    assert_eq!(RangeFacts::new(&g).range_of(n), None);

    // A fixed unused division still observes a nonzero count's poison.
    let mut g = graph(
        Opcode::BitCount,
        P::TRAILING_ZEROS | P::ZERO_POISON,
        IntegerWidth::W8,
        false,
    );
    let n = intrinsic_node(&g);
    let zero = g.add_constant(0, g.rust_type(g.input(n, 0)).unwrap());
    g.set_input(n, 0, zero);
    let ret = g.ids().find(|&n| g.op(n) == Opcode::Return).unwrap();
    let two = g.add_constant(2, g.rust_type(n).unwrap());
    g.add_node(
        Opcode::DivUnsigned,
        g.control(ret),
        &[n, two],
        0,
        g.value_type(n).clone(),
    );
    g.set_input(ret, 1, two);
    verify_graph(&mut g).unwrap();
    assert_eq!(run(&g, &[0]), Err("poison"));
    optimize_specialized_graph(&mut g).unwrap();
    assert_eq!(run(&g, &[0]), Err("poison"));
    assert!(g.ids().any(|n| g.op(n) == Opcode::DivUnsigned));
}

struct Oracle {
    types: RecordedTypes,
    abi: FunctionAbi,
}
impl SpecializationOracle for Oracle {
    fn instantiate_type(&mut self, ty: TyId, into: &mut RecordedTypes) -> Result<TyId, String> {
        Ok(into.intern(self.types.shape(ty).clone(), self.types.size_align(ty)))
    }
    fn answer(&mut self, _: &Hole) -> Result<HoleAnswer, String> {
        Err("no intrinsic holes".into())
    }
    fn function_abi(&mut self) -> Result<FunctionAbi, String> {
        Ok(self.abi.clone())
    }
}

#[test]
fn r_execution_and_specialization_preserve_intrinsic_payloads() {
    for (op, payload, args) in [
        (Opcode::BitCount, 1, vec![0]),
        (Opcode::BitCount, 5, vec![0]),
        (Opcode::FunnelShift, 3, vec![17, 29, 0]),
        (Opcode::FunnelShift, 3, vec![17, 29, 256]),
        (Opcode::ByteSwap, 0, vec![0x1234]),
        (Opcode::SaturatingAdd, 0, vec![65535, 1]),
    ] {
        let mut g = graph(op, payload, IntegerWidth::W16, false);
        let n = intrinsic_node(&g);
        let ret = g.rust_type(n).unwrap();
        let signature = RustSignature {
            args: g
                .inputs(n)
                .iter()
                .map(|&v| g.rust_type(v).unwrap())
                .collect(),
            ret,
        };
        let expected = run(&g, &args);
        g.level = GraphLevel::Rust;
        g.rust_signature = Some(signature);
        verify_graph(&mut g).unwrap();
        let mut oracle = Oracle {
            types: g.oracle.clone(),
            abi: g.abi.clone(),
        };
        let result = Interpreter::new(&g).run_rust_values(
            &args.iter().copied().map(Value::Scalar).collect::<Vec<_>>(),
            &mut |_: &str, _: &[Value]| unreachable!(),
            &mut oracle,
        );
        match expected {
            Ok(value) => assert_eq!(result, Ok(SemanticOutcome::Return(Value::Scalar(value)))),
            Err(_) => assert!(matches!(result, Err(InterpretError::PoisonObserved(_)))),
        }
        let definition = RustDefinition::prepare(g).unwrap();
        let specialized = definition
            .specialize("intrinsic_instance", &mut oracle)
            .unwrap();
        assert_eq!(run(&specialized.graph, &args), expected);
        assert_eq!(
            specialized
                .graph
                .payload(intrinsic_node(&specialized.graph)),
            payload
        );
    }
}

#[test]
fn body_producer_keeps_a_single_intrinsic_and_count_result_u32() {
    for op in [
        IntegerIntrinsic::BitCount {
            kind: BitCountKind::LeadingZeros,
            zero_poison: false,
        },
        IntegerIntrinsic::ByteSwap,
        IntegerIntrinsic::BitReverse,
        IntegerIntrinsic::FunnelShift {
            direction: FunnelDirection::Right,
            amount_in_range: true,
        },
        IntegerIntrinsic::SaturatingAdd,
        IntegerIntrinsic::SaturatingSub,
    ] {
        let mut types = RecordedTypes::new();
        let word = types.intern(
            TypeShape::Int {
                width: IntegerWidth::W64,
                signed: false,
            },
            Some(SizeAlign { size: 8, align: 8 }),
        );
        let u32_ = types.intern(
            TypeShape::Int {
                width: IntegerWidth::W32,
                signed: false,
            },
            Some(SizeAlign { size: 4, align: 4 }),
        );
        let (opcode, payload) = P::from_body(&op);
        let arity = opcode.integer_intrinsic_arity().unwrap();
        let abi = FunctionAbi {
            args: vec![PassMode::Direct; arity],
            ret: PassMode::Direct,
            c_variadic: false,
        };
        let mut b = BodyBuilder::new("body_intrinsic", abi, types);
        let ret = b.add_local(LocalDecl {
            ty: if opcode == Opcode::BitCount {
                u32_
            } else {
                word
            },
            mutability: Mutability::Mut,
            name: None,
        });
        let args = (0..arity)
            .map(|i| {
                let l = b.add_local(LocalDecl {
                    ty: if opcode == Opcode::FunnelShift && i == 2 {
                        u32_
                    } else {
                        word
                    },
                    mutability: Mutability::Not,
                    name: None,
                });
                Operand::Copy(Place::local(l))
            })
            .collect();
        b.set_arg_count(arity);
        b.push_statement(StatementKind::Assign(
            Place::local(ret),
            Rvalue::IntegerIntrinsic(op, args),
        ));
        b.finish_block(Terminator::Return, false);
        let mut g = build_graph(&b.finish()).unwrap();
        verify_graph(&mut g).unwrap();
        assert_eq!(g.ids().filter(|&n| g.op(n) == opcode).count(), 1);
        assert_eq!(g.payload(intrinsic_node(&g)), payload);
    }
}
