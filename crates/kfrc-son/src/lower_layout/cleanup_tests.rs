use super::*;
use crate::{CallTarget, InterpretError, Interpreter, Outcome, verify_graph};
use kfrc_body::{FunctionAbi, Mutability, PassMode, RecordedTypes};

struct Fixture {
    graph: FunctionGraph,
    word: TyId,
    boolean: TyId,
    pointer: TyId,
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
                signed: true,
            },
            Some(SizeAlign { size: 8, align: 8 }),
        );
        let boolean = types.intern(TypeShape::Bool, Some(SizeAlign { size: 1, align: 1 }));
        let pointer = types.intern(
            TypeShape::RawPointer {
                mutability: Mutability::Mut,
                pointee: word,
            },
            Some(SizeAlign { size: 8, align: 8 }),
        );
        let mut graph = FunctionGraph::new(
            "layout_cleanup",
            FunctionAbi {
                args: vec![PassMode::Direct; 3],
                ret: PassMode::Direct,
                c_variadic: false,
            },
            types,
        );
        graph.level = GraphLevel::Layout;
        let start = graph.add_node(
            Opcode::Start,
            NodeId::NONE,
            &[],
            0,
            ValueType::Tuple(vec![
                ValueType::Control,
                ValueType::Effect,
                ValueType::Rust(pointer),
                ValueType::Rust(pointer),
                ValueType::Rust(boolean),
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
        let arg_types = [pointer, pointer, boolean];
        let args = std::array::from_fn(|i| {
            graph.add_node(
                Opcode::Projection,
                NodeId::NONE,
                &[start],
                i as u32 + 2,
                ValueType::Rust(arg_types[i]),
            )
        });
        Self {
            graph,
            word,
            boolean,
            pointer,
            control,
            effect,
            args,
        }
    }

    fn address(&mut self, base: NodeId, offset: u128) -> NodeId {
        let offset = self.graph.add_constant(offset, self.word);
        self.graph.add_node(
            Opcode::PtrOffset,
            NodeId::NONE,
            &[base, offset],
            0,
            ValueType::Rust(self.pointer),
        )
    }

    fn branch(&mut self, condition: NodeId) -> [NodeId; 2] {
        let branch = self.graph.add_node(
            Opcode::If,
            self.control,
            &[condition],
            0,
            ValueType::Tuple(vec![ValueType::Control, ValueType::Control]),
        );
        std::array::from_fn(|i| {
            self.graph.add_node(
                Opcode::Projection,
                NodeId::NONE,
                &[branch],
                i as u32,
                ValueType::Control,
            )
        })
    }

    fn return_at(&mut self, control: NodeId, value: NodeId) {
        self.graph.add_node(
            Opcode::Return,
            control,
            &[self.effect, value],
            0,
            ValueType::Control,
        );
    }

    fn call_before_branch(&mut self) {
        let callee = self.graph.add_callee(CallTarget {
            symbol: "before".into(),
            abi: FunctionAbi {
                args: vec![],
                ret: PassMode::Ignore,
                c_variadic: false,
            },
        });
        let call = self.graph.add_node(
            Opcode::Call,
            self.control,
            &[self.effect],
            callee,
            ValueType::Tuple(vec![ValueType::Control, ValueType::Effect]),
        );
        self.control = self.graph.add_node(
            Opcode::Projection,
            NodeId::NONE,
            &[call],
            0,
            ValueType::Control,
        );
        self.effect = self.graph.add_node(
            Opcode::Projection,
            NodeId::NONE,
            &[call],
            1,
            ValueType::Effect,
        );
    }
}

fn evaluate(
    graph: &FunctionGraph,
    a: u64,
    b: u64,
    flag: u64,
) -> (Result<Outcome, InterpretError>, Vec<String>) {
    let mut calls = Vec::new();
    let outcome = Interpreter::new(graph).run(&[a, b, flag], &mut |name: &str, _: &[u64]| {
        calls.push(name.to_owned());
        Ok(0)
    });
    (outcome, calls)
}

#[test]
fn address_numbering_keeps_identity_types_and_control_scopes() {
    let mut f = Fixture::new();
    let a = f.address(f.args[0], 3);
    let zero = f.address(f.args[0], 0);
    let duplicate = f.address(zero, 3);
    let other_base = f.address(f.args[1], 3);
    let other_offset = f.address(f.args[0], u128::from(u64::MAX));
    let narrow = f.graph.oracle.intern(
        TypeShape::Int {
            width: IntegerWidth::W32,
            signed: true,
        },
        Some(SizeAlign { size: 4, align: 4 }),
    );
    let three = f.graph.add_constant(3, narrow);
    let other_width = f.graph.add_node(
        Opcode::PtrOffset,
        NodeId::NONE,
        &[f.args[0], three],
        0,
        ValueType::Rust(f.pointer),
    );
    let other_pointer = f.graph.oracle.intern(
        TypeShape::RawPointer {
            mutability: Mutability::Not,
            pointee: f.word,
        },
        Some(SizeAlign { size: 8, align: 8 }),
    );
    let offset = f.graph.input(a, 1);
    let other_type = f.graph.add_node(
        Opcode::PtrOffset,
        NodeId::NONE,
        &[f.args[0], offset],
        0,
        ValueType::Rust(other_pointer),
    );
    let anchored = f.graph.add_node(
        Opcode::PtrOffset,
        f.control,
        &[f.args[0], offset],
        0,
        ValueType::Rust(f.pointer),
    );
    let compare = f.graph.add_node(
        Opcode::Compare,
        NodeId::NONE,
        &[a, duplicate],
        P::Equal as u32,
        ValueType::Rust(f.boolean),
    );
    f.return_at(f.control, compare);
    verify_graph(&mut f.graph).unwrap();
    let before = f.graph.clone();
    address_cse::run(&mut f.graph);
    verify_graph(&mut f.graph).unwrap();
    assert_eq!(f.graph.resolve(a), f.graph.resolve(duplicate));
    for other in [other_base, other_offset, other_width, other_type, anchored] {
        assert_ne!(f.graph.resolve(a), f.graph.resolve(other));
    }
    for base in [0, 7, u64::MAX - 1] {
        assert_eq!(
            evaluate(&before, base, base, 0),
            evaluate(&f.graph, base, base, 0)
        );
        assert_eq!(evaluate(&f.graph, base, base, 0).0, Ok(Outcome::Return(1)));
    }
}

#[test]
fn known_boolean_branches_preserve_calls_and_other_value_users() {
    for taken in [false, true] {
        let mut f = Fixture::new();
        f.call_before_branch();
        let mask = f.graph.add_constant(u128::from(taken), f.boolean);
        let condition = f.graph.add_node(
            if taken { Opcode::BitOr } else { Opcode::BitAnd },
            NodeId::NONE,
            &[f.args[2], mask],
            0,
            ValueType::Rust(f.boolean),
        );
        let [yes, no] = f.branch(condition);
        // The original condition also reaches the return; branch refinement
        // must not globally substitute a possibly poison-producing expression.
        f.return_at(yes, condition);
        f.return_at(no, condition);
        verify_graph(&mut f.graph).unwrap();
        let before = f.graph.clone();
        run(&mut f.graph).unwrap();
        verify_graph(&mut f.graph).unwrap();
        assert!(!f.graph.ids().any(|n| f.graph.op(n) == Opcode::If));
        for flag in [0, 1] {
            assert_eq!(
                evaluate(&before, 0, 0, flag),
                evaluate(&f.graph, 0, 0, flag)
            );
            assert_eq!(
                evaluate(&f.graph, 0, 0, flag),
                (Ok(Outcome::Return(u64::from(taken))), vec!["before".into()])
            );
        }
    }
}

#[test]
fn branch_refinement_neither_speculates_nor_erases_trapping_division() {
    for trap_on_taken in [false, true] {
        for (numerator, divisor) in [(1u64, 0u64), (1u64 << 63, u64::MAX)] {
            let mut f = Fixture::new();
            let zero = f.graph.add_constant(0, f.boolean);
            let condition = f.graph.add_node(
                Opcode::BitAnd,
                NodeId::NONE,
                &[f.args[2], zero],
                0,
                ValueType::Rust(f.boolean),
            );
            let [yes, no] = f.branch(condition);
            let numerator = f.graph.add_constant(u128::from(numerator), f.word);
            let divisor = f.graph.add_constant(u128::from(divisor), f.word);
            f.graph.add_node(
                Opcode::DivSigned,
                if trap_on_taken { no } else { yes },
                &[numerator, divisor],
                0,
                ValueType::Rust(f.word),
            );
            f.return_at(yes, numerator);
            f.return_at(no, numerator);
            verify_graph(&mut f.graph).unwrap();
            let before = f.graph.clone();
            run(&mut f.graph).unwrap();
            verify_graph(&mut f.graph).unwrap();
            for flag in [0, 1] {
                let expected = evaluate(&before, 0, 0, flag);
                let actual = evaluate(&f.graph, 0, 0, flag);
                if trap_on_taken {
                    assert!(matches!(
                        expected.0,
                        Err(InterpretError::UndefinedBehaviour(_))
                    ));
                    assert!(matches!(
                        actual.0,
                        Err(InterpretError::UndefinedBehaviour(_))
                    ));
                } else {
                    assert_eq!(expected, actual);
                    assert!(actual.0.is_ok());
                }
            }
        }
    }
}

#[test]
fn unknown_and_undefined_boolean_conditions_are_not_invented() {
    for poison in [false, true] {
        let mut f = Fixture::new();
        let condition = if poison {
            f.graph.add_node(
                Opcode::Undefined,
                NodeId::NONE,
                &[],
                0,
                ValueType::Rust(f.boolean),
            )
        } else {
            f.args[2]
        };
        let [yes, no] = f.branch(condition);
        let one = f.graph.add_constant(1, f.word);
        let zero = f.graph.add_constant(0, f.word);
        f.return_at(yes, one);
        f.return_at(no, zero);
        verify_graph(&mut f.graph).unwrap();
        bit_conditions(&mut f.graph);
        verify_graph(&mut f.graph).unwrap();
        assert!(f.graph.ids().any(|n| f.graph.op(n) == Opcode::If));
        for flag in [0, 1] {
            let outcome = evaluate(&f.graph, 0, 0, flag).0;
            if poison {
                assert!(matches!(outcome, Err(InterpretError::PoisonObserved(_))));
            } else {
                assert_eq!(outcome, Ok(Outcome::Return(flag)));
            }
        }
    }
}
