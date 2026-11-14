//! Source assumptions remain fixed requirements, distinct from runtime checks.
use kfrc_body::verify::{BodyError, verify_body};
use kfrc_body::*;
use kfrc_son::{
    InterpretError, Interpreter, Opcode, SemanticOutcome, Value, ValueType, build_graph,
    verify_graph,
};

fn local(b: &mut BodyBuilder, ty: TyId) -> LocalId {
    b.add_local(LocalDecl {
        ty,
        mutability: Mutability::Mut,
        name: None,
    })
}
fn constant(b: &mut BodyBuilder, ty: TyId, bits: u128) -> Operand {
    Operand::Constant(b.add_constant(Constant {
        ty,
        kind: ConstantKind::Scalar(bits),
    }))
}
fn base() -> (BodyBuilder, LocalId, TyId, TyId) {
    let mut types = RecordedTypes::new();
    let word = types.intern(
        TypeShape::Int {
            width: IntegerWidth::Pointer,
            signed: false,
        },
        Some(SizeAlign { size: 8, align: 8 }),
    );
    let boolean = types.intern(TypeShape::Bool, Some(SizeAlign { size: 1, align: 1 }));
    let mut b = BodyBuilder::new(
        "assumption",
        FunctionAbi {
            args: vec![],
            ret: PassMode::Direct,
            c_variadic: false,
        },
        types,
    );
    let ret = local(&mut b, word);
    (b, ret, word, boolean)
}
fn answer(b: &mut BodyBuilder, ret: LocalId, word: TyId, bits: u128) {
    let value = constant(b, word, bits);
    b.push_statement(StatementKind::Assign(Place::local(ret), Rvalue::Use(value)));
}
fn run(body: Body) -> Result<SemanticOutcome, InterpretError> {
    let mut graph = build_graph(&body).unwrap();
    verify_graph(&mut graph).unwrap();
    Interpreter::new(&graph).run_values(&[], &mut |_: &str, _: &[Value]| unreachable!())
}

#[test]
fn true_false_and_poison_assumptions_execute_even_without_a_value_result() {
    for input in [Some(true), Some(false), None] {
        let (mut b, ret, word, boolean) = base();
        let predicate = if let Some(value) = input {
            constant(&mut b, boolean, u128::from(value))
        } else {
            Operand::Copy(Place::local(local(&mut b, boolean)))
        };
        b.push_statement(StatementKind::Assume(predicate));
        answer(&mut b, ret, word, 42);
        b.finish_block(Terminator::Return, false);
        let body = b.finish();
        assert!(body.to_string().contains("assume("));
        let mut graph = build_graph(&body).unwrap();
        verify_graph(&mut graph).unwrap();
        let assume = graph
            .ids()
            .find(|&n| graph.op(n) == Opcode::Assume)
            .unwrap();
        assert_eq!(
            graph.value_type(assume),
            &ValueType::Tuple(vec![ValueType::Control, ValueType::Effect])
        );
        assert!(graph.find_projection(assume, 2).is_none());
        let result =
            Interpreter::new(&graph).run_values(&[], &mut |_: &str, _: &[Value]| unreachable!());
        match input {
            Some(true) => assert_eq!(result, Ok(SemanticOutcome::Return(Value::Scalar(42)))),
            Some(false) | None => assert!(matches!(
                result,
                Err(InterpretError::UndefinedBehaviour(_)) | Err(InterpretError::PoisonObserved(_))
            )),
        }
    }
}

#[test]
fn runtime_check_failure_returns_its_error_instead_of_becoming_an_assumption() {
    for value in [false, true] {
        let (mut b, ret, word, boolean) = base();
        let condition = constant(&mut b, boolean, u128::from(value));
        b.finish_block(
            Terminator::SwitchInt {
                discriminant: condition,
                targets: SwitchTargets {
                    cases: vec![(1, BlockId(1))],
                    otherwise: BlockId(2),
                },
            },
            false,
        );
        answer(&mut b, ret, word, 42);
        b.finish_block(Terminator::Return, false);
        answer(&mut b, ret, word, 99);
        b.finish_block(Terminator::Return, false);
        let body = b.finish();
        let graph = build_graph(&body).unwrap();
        assert!(!graph.ids().any(|n| graph.op(n) == Opcode::Assume));
        assert_eq!(
            run(body),
            Ok(SemanticOutcome::Return(Value::Scalar(if value {
                42
            } else {
                99
            })))
        );
    }
}

#[test]
fn assume_move_deinitializes_its_place_and_dynamic_index_materializes_storage() {
    for move_value in [false, true] {
        let (mut b, ret, word, boolean) = base();
        let array = b.types().intern(
            TypeShape::Array {
                element: boolean,
                length: 1,
            },
            Some(SizeAlign { size: 1, align: 1 }),
        );
        let values = local(&mut b, array);
        let index = local(&mut b, word);
        let yes = constant(&mut b, boolean, 1);
        b.push_statement(StatementKind::Assign(
            Place::local(values),
            Rvalue::Aggregate(AggregateKind::Array(boolean), vec![yes]),
        ));
        let zero = constant(&mut b, word, 0);
        b.push_statement(StatementKind::Assign(
            Place::local(index),
            Rvalue::Use(zero),
        ));
        let selected = Place {
            local: values,
            projection: b.add_projection(vec![ProjectionElem::Index(index)]),
        };
        b.push_statement(StatementKind::Assume(if move_value {
            Operand::Move(selected)
        } else {
            Operand::Copy(selected)
        }));
        b.push_statement(StatementKind::Assume(Operand::Copy(selected)));
        answer(&mut b, ret, word, 42);
        b.finish_block(Terminator::Return, false);
        let body = b.finish();
        let mut graph = build_graph(&body).unwrap();
        verify_graph(&mut graph).unwrap();
        assert!(graph.ids().any(|n| graph.op(n) == Opcode::LocalSlot));
        assert!(graph.ids().any(|n| graph.op(n) == Opcode::IndexAddr));
        let result = run(body);
        if move_value {
            assert!(matches!(
                result,
                Err(InterpretError::PoisonObserved(_)) | Err(InterpretError::UndefinedBehaviour(_))
            ));
        } else {
            assert_eq!(result, Ok(SemanticOutcome::Return(Value::Scalar(42))));
        }
    }
}

#[test]
fn nonboolean_assumption_is_rejected_by_body_validation() {
    let (mut b, ret, word, _) = base();
    let integer = constant(&mut b, word, 1);
    b.push_statement(StatementKind::Assume(integer));
    answer(&mut b, ret, word, 42);
    b.finish_block(Terminator::Return, false);
    assert!(matches!(
        verify_body(&b.finish()),
        Err(BodyError::NonBooleanAssume { .. })
    ));
}
