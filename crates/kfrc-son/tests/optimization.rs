use kfrc_body::{FunctionAbi, IntegerWidth, PassMode, RecordedTypes, SizeAlign, TyId, TypeShape};
use kfrc_son::{
    ComparePredicate, FunctionGraph, InterpretError, Interpreter, NodeId, Opcode, ValueType,
    optimize_graph, verify_graph,
};

fn graph(width: IntegerWidth, signed: bool) -> (FunctionGraph, TyId, TyId, NodeId, NodeId) {
    let mut types = RecordedTypes::new();
    let ty = types.intern(
        TypeShape::Int { width, signed },
        Some(SizeAlign {
            size: u64::from(width.bits(64)) / 8,
            align: 1,
        }),
    );
    let boolean = types.intern(TypeShape::Bool, Some(SizeAlign { size: 1, align: 1 }));
    let mut graph = FunctionGraph::new(
        "optimized",
        FunctionAbi {
            args: Vec::new(),
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
        ValueType::Tuple(vec![ValueType::Control, ValueType::Effect]),
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
    (graph, ty, boolean, control, effect)
}

fn run(graph: &FunctionGraph) -> Result<kfrc_son::Outcome, InterpretError> {
    Interpreter::new(&layout_graph(graph)).run(&[], &mut |name: &str, _: &[u64]| {
        Err(InterpretError::UnknownCallee(name.into()))
    })
}

#[test]
fn folded_integer_graphs_preserve_width_sign_shifts_and_comparisons() {
    let ops = [
        Opcode::Add,
        Opcode::Sub,
        Opcode::Mul,
        Opcode::BitAnd,
        Opcode::BitOr,
        Opcode::BitXor,
        Opcode::ShiftLeft,
        Opcode::ShiftRightLogical,
        Opcode::ShiftRightArithmetic,
        Opcode::Negate,
        Opcode::BitNot,
        Opcode::Compare,
    ];
    for width in [
        IntegerWidth::W8,
        IntegerWidth::W16,
        IntegerWidth::W32,
        IntegerWidth::W64,
    ] {
        for signed in [false, true] {
            let mut seed = 0xbadc0ffee_u64;
            for sample in 0..24 {
                seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
                let left = if sample < 4 {
                    [0, 1, u64::MAX, 1u64 << (width.bits(64) - 1)][sample]
                } else {
                    seed
                };
                let right = seed.rotate_left(19);
                for op in ops {
                    let (mut graph, ty, boolean, control, effect) = graph(width, signed);
                    let mask = u64::MAX >> (64 - width.bits(64));
                    let lhs = graph.add_constant(u128::from(left & mask), ty);
                    let rhs = graph.add_constant(u128::from(right & mask), ty);
                    let result_ty = if op == Opcode::Compare { boolean } else { ty };
                    let inputs = if matches!(op, Opcode::Negate | Opcode::BitNot) {
                        vec![lhs]
                    } else {
                        vec![lhs, rhs]
                    };
                    let predicate = ComparePredicate::ALL[sample % ComparePredicate::ALL.len()];
                    let result = graph.add_node(
                        op,
                        NodeId::NONE,
                        &inputs,
                        if op == Opcode::Compare {
                            predicate as u32
                        } else {
                            0
                        },
                        ValueType::Rust(result_ty),
                    );
                    graph.add_node(
                        Opcode::Return,
                        control,
                        &[effect, result],
                        0,
                        ValueType::Control,
                    );
                    verify_graph(&mut graph).unwrap();
                    let expected = run(&graph);
                    let stats = optimize_graph(&mut graph);
                    verify_graph(&mut graph).unwrap();
                    assert_eq!(
                        run(&graph),
                        expected,
                        "{op:?} {width:?} signed={signed} sample={sample}"
                    );
                    assert_eq!(stats.constants_folded, 1);
                    assert!(!graph.ids().any(|node| graph.op(node) == op));
                }
            }
        }
    }
}

#[test]
fn constant_cast_extends_from_the_source_signedness() {
    for signed in [false, true] {
        let (mut graph, source_ty, _, control, effect) = graph(IntegerWidth::W8, signed);
        let target_ty = graph.oracle.intern(
            TypeShape::Int {
                width: IntegerWidth::W64,
                signed: false,
            },
            Some(SizeAlign { size: 8, align: 8 }),
        );
        let source = graph.add_constant(255, source_ty);
        let cast = graph.add_node(
            Opcode::IntCast,
            NodeId::NONE,
            &[source],
            0,
            ValueType::Rust(target_ty),
        );
        graph.add_node(
            Opcode::Return,
            control,
            &[effect, cast],
            0,
            ValueType::Control,
        );
        verify_graph(&mut graph).unwrap();
        let before = run(&graph);
        optimize_graph(&mut graph);
        verify_graph(&mut graph).unwrap();
        assert_eq!(run(&graph), before);
        assert_eq!(
            before.unwrap(),
            kfrc_son::Outcome::Return(if signed { u64::MAX } else { 255 })
        );
    }
}

#[test]
fn identities_preserve_poison_and_do_not_simplify_multiply_by_zero() {
    for op in [Opcode::Add, Opcode::Mul] {
        let (mut graph, ty, _, control, effect) = graph(IntegerWidth::W64, false);
        let poison = graph.add_node(Opcode::Undefined, NodeId::NONE, &[], 0, ValueType::Rust(ty));
        let zero = graph.add_constant(0, ty);
        let result = graph.add_node(op, NodeId::NONE, &[poison, zero], 0, ValueType::Rust(ty));
        graph.add_node(
            Opcode::Return,
            control,
            &[effect, result],
            0,
            ValueType::Control,
        );
        verify_graph(&mut graph).unwrap();
        assert!(matches!(
            run(&graph),
            Err(InterpretError::PoisonObserved(_))
        ));
        let stats = optimize_graph(&mut graph);
        verify_graph(&mut graph).unwrap();
        assert!(matches!(
            run(&graph),
            Err(InterpretError::PoisonObserved(_))
        ));
        assert_eq!(stats.identities_simplified, usize::from(op == Opcode::Add));
    }
}

#[test]
fn trapping_operations_stay_on_the_control_path() {
    let (mut graph, ty, _, control, effect) = graph(IntegerWidth::W64, false);
    let lhs = graph.add_constant(7, ty);
    let zero = graph.add_constant(0, ty);
    let quotient = graph.add_node(
        Opcode::DivUnsigned,
        control,
        &[lhs, zero],
        0,
        ValueType::Rust(ty),
    );
    graph.add_node(
        Opcode::Return,
        control,
        &[effect, quotient],
        0,
        ValueType::Control,
    );
    verify_graph(&mut graph).unwrap();
    assert!(matches!(
        run(&graph),
        Err(InterpretError::UndefinedBehaviour(_))
    ));
    optimize_graph(&mut graph);
    verify_graph(&mut graph).unwrap();
    assert!(
        graph
            .ids()
            .any(|node| graph.op(node) == Opcode::DivUnsigned)
    );
    assert!(matches!(
        run(&graph),
        Err(InterpretError::UndefinedBehaviour(_))
    ));
}

/// Exercise the unique value pipeline before interpreting its physical ABI.
fn layout_graph(graph: &kfrc_son::FunctionGraph) -> kfrc_son::FunctionGraph {
    let mut graph = graph.clone();
    kfrc_son::verify_graph(&mut graph).unwrap();
    kfrc_son::lower_layout(&mut graph).unwrap();
    kfrc_son::verify_graph(&mut graph).unwrap();
    graph
}
