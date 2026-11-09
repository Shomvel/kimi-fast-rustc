use kfrc_body::{
    FunctionAbi, IntegerWidth, PassMode, RecordedTypes, SemanticAggregate, SizeAlign, TyId,
    TypeShape,
};
use kfrc_son::interpret::{SemanticOutcome, Value};
use kfrc_son::{
    CallTarget, FunctionGraph, GraphLevel, InlineBudget, InlineSkip, InterpretError, Interpreter,
    NodeId, Opcode, ValueType, fold_identity_calls, inline_calls, verify_graph,
};
use std::collections::HashMap;

struct Function {
    graph: FunctionGraph,
    int: TyId,
    control: NodeId,
    effect: NodeId,
    args: Vec<NodeId>,
}

fn shell(f: &mut Function) {
    let jump = f
        .graph
        .add_node(Opcode::Jump, f.control, &[], 0, ValueType::Control);
    let region = f
        .graph
        .add_node(Opcode::Region, NodeId::NONE, &[jump], 0, ValueType::Control);
    f.effect = f.graph.add_node(
        Opcode::Phi,
        region,
        &[region, f.effect],
        0,
        ValueType::Effect,
    );
    f.control = region;
}

fn execute(
    graph: &FunctionGraph,
    functions: &HashMap<String, FunctionGraph>,
    args: &[Value],
) -> Result<SemanticOutcome, InterpretError> {
    Interpreter::new(graph).run_values(args, &mut |symbol: &str, values: &[Value]| {
        let source = functions
            .get(symbol)
            .ok_or_else(|| InterpretError::UnknownCallee(symbol.into()))?;
        match execute(source, functions, values)? {
            SemanticOutcome::Return(value) => Ok(value),
            SemanticOutcome::Unreachable => Err(InterpretError::UndefinedBehaviour(
                "unreachable callee".into(),
            )),
        }
    })
}

#[test]
fn exact_parameter_return_folds_with_zero_inline_budget_and_independent_types() {
    for wrapped in [false, true] {
        let mut callee = Function::new("arbitrary_identity_name", 2, true);
        if wrapped {
            shell(&mut callee);
            shell(&mut callee);
        }
        callee.ret(callee.args[1]);
        callee.verify();
        let mut caller = Function::new("caller", 2, false);
        let result = caller.call(&callee.graph, &caller.args.clone(), caller.int);
        caller.ret(result);
        caller.verify();
        let functions = HashMap::from([(callee.graph.symbol.clone(), callee.graph)]);
        let before = caller.graph.clone();
        let budget = InlineBudget {
            max_depth: 0,
            max_nodes: 0,
            max_calls: 0,
        };
        assert_eq!(
            inline_calls(&mut caller.graph, &functions, budget)
                .unwrap()
                .calls_inlined,
            0
        );
        let stats = fold_identity_calls(&mut caller.graph, &functions, None).unwrap();
        assert_eq!(stats.calls_folded, 1);
        assert_eq!(
            inline_calls(&mut caller.graph, &functions, budget)
                .unwrap()
                .calls_inlined,
            0
        );
        assert_eq!(
            caller
                .graph
                .ids()
                .filter(|&n| caller.graph.op(n) == Opcode::RequireDefined)
                .count(),
            3
        );
        assert!(
            !caller
                .graph
                .ids()
                .any(|n| caller.graph.op(n) == Opcode::Call)
        );
        for args in [[0, 0], [17, 42], [u64::MAX, 1]] {
            let args: Vec<_> = args.into_iter().map(Value::Scalar).collect();
            assert_eq!(
                execute(&before, &functions, &args),
                execute(&caller.graph, &functions, &args)
            );
        }
    }
}

#[test]
fn poison_in_nonreturned_or_returned_argument_traps_between_effectful_calls() {
    for poisoned in [0, 1] {
        let mut callee = Function::new("identity", 2, false);
        callee.ret(callee.args[1]);
        callee.verify();
        let mut caller = Function::new("caller", 0, false);
        let mut event = Function::new("before", 0, false);
        let constant = event.graph.add_constant(0, event.int);
        event.ret(constant);
        caller.call(&event.graph, &[], caller.int);
        let poison = caller.node(Opcode::Undefined, &[], 0, caller.int);
        let value = caller.graph.add_constant(7, caller.int);
        let args = if poisoned == 0 {
            [poison, value]
        } else {
            [value, poison]
        };
        caller.call(&callee.graph, &args, caller.int);
        event.graph.symbol = "after".into();
        caller.call(&event.graph, &[], caller.int);
        caller.ret(value); // The identity result is deliberately unused.
        caller.verify();
        let functions = HashMap::from([("identity".into(), callee.graph)]);
        for optimized in [false, true] {
            if optimized {
                assert_eq!(
                    fold_identity_calls(&mut caller.graph, &functions, None)
                        .unwrap()
                        .calls_folded,
                    1
                );
            }
            let mut events = Vec::new();
            let result = Interpreter::new(&caller.graph).run_values(
                &[],
                &mut |symbol: &str, values: &[Value]| {
                    if symbol == "identity" {
                        return Ok(values[1].clone());
                    }
                    events.push(symbol.to_owned());
                    Ok(Value::Scalar(0))
                },
            );
            assert!(matches!(result, Err(InterpretError::PoisonObserved(_))));
            assert_eq!(events, ["before"]);
        }
    }
}

#[test]
fn successful_fold_preserves_surrounding_call_order() {
    let mut callee = Function::new("identity", 1, false);
    callee.ret(callee.args[0]);
    callee.verify();
    let mut caller = Function::new("caller", 0, false);
    let mut event = Function::new("before", 0, false);
    let value = caller.call(&event.graph, &[], caller.int);
    let result = caller.call(&callee.graph, &[value], caller.int);
    event.graph.symbol = "after".into();
    caller.call(&event.graph, &[], caller.int);
    caller.ret(result);
    caller.verify();
    let functions = HashMap::from([("identity".into(), callee.graph)]);
    for optimized in [false, true] {
        if optimized {
            fold_identity_calls(&mut caller.graph, &functions, None).unwrap();
        }
        let mut events = Vec::new();
        let result = Interpreter::new(&caller.graph)
            .run_values(&[], &mut |symbol: &str, values: &[Value]| {
                if symbol == "identity" {
                    return Ok(values[0].clone());
                }
                events.push(symbol.to_owned());
                Ok(Value::Scalar(42))
            })
            .unwrap();
        assert_eq!(result, SemanticOutcome::Return(Value::Scalar(42)));
        assert_eq!(events, ["before", "after"]);
    }
}

#[test]
fn recursive_identity_looking_callee_and_type_or_abi_mismatch_are_skipped() {
    for kind in ["recursive", "type", "abi", "compute"] {
        let mut callee = Function::new("identity", 1, false);
        if kind == "recursive" {
            let declaration = callee.graph.clone();
            callee.call(&declaration, &callee.args.clone(), callee.int);
        }
        let returned = if kind == "compute" {
            let one = callee.graph.add_constant(1, callee.int);
            callee.node(Opcode::Add, &[callee.args[0], one], 0, callee.int)
        } else {
            callee.args[0]
        };
        callee.ret(returned);
        callee.verify();
        let mut caller = Function::new("caller", 1, false);
        let result = caller.call(&callee.graph, &caller.args.clone(), caller.int);
        caller.ret(result);
        caller.verify();
        if kind == "abi" {
            callee.graph.abi.args[0] = PassMode::Ignore;
        }
        if kind == "type" {
            let signed = callee.graph.oracle.intern(
                TypeShape::Int {
                    width: IntegerWidth::W64,
                    signed: true,
                },
                Some(SizeAlign { size: 8, align: 8 }),
            );
            callee.graph.set_value_type(
                callee.graph.start,
                ValueType::Tuple(vec![
                    ValueType::Control,
                    ValueType::Effect,
                    ValueType::Rust(signed),
                ]),
            );
            callee
                .graph
                .set_value_type(callee.args[0], ValueType::Rust(signed));
        }
        let functions = HashMap::from([("identity".into(), callee.graph)]);
        let before = caller.graph.dump();
        let stats = fold_identity_calls(&mut caller.graph, &functions, None).unwrap();
        assert_eq!(stats.calls_folded, 0, "{kind}");
        assert_eq!(caller.graph.dump(), before);
        assert_eq!(
            stats.skipped[0].1,
            match kind {
                "abi" => InlineSkip::AbiMismatch,
                "type" => InlineSkip::TypeConflict,
                _ => InlineSkip::Unsupported,
            }
        );
    }
}

#[test]
fn enum_identity_observes_only_the_active_payload() {
    for active_poison in [false, true] {
        let mut callee = Function::new("enum_identity", 1, false);
        let enumeration = callee.graph.oracle.intern(
            TypeShape::Adt {
                name: "Choice".into(),
            },
            None,
        );
        callee.graph.oracle.record_semantic_aggregate(
            enumeration,
            SemanticAggregate {
                fields: vec![],
                variants: vec![vec![], vec![callee.int]],
            },
        );
        callee
            .graph
            .oracle
            .record_semantic_enum(enumeration, vec![0, 1]);
        callee.graph.set_value_type(
            callee.graph.start,
            ValueType::Tuple(vec![
                ValueType::Control,
                ValueType::Effect,
                ValueType::Rust(enumeration),
            ]),
        );
        callee
            .graph
            .set_value_type(callee.args[0], ValueType::Rust(enumeration));
        callee.ret(callee.args[0]);
        callee.verify();
        let mut caller = Function::new("caller", 0, false);
        caller.graph.oracle = callee.graph.oracle.clone();
        let poison = caller.node(Opcode::Undefined, &[], 0, caller.int);
        let args = if active_poison { vec![poison] } else { vec![] };
        let value = caller.node(
            Opcode::MakeVariant,
            &args,
            u32::from(active_poison),
            enumeration,
        );
        let returned = caller.call(&callee.graph, &[value], enumeration);
        caller.ret(returned);
        caller.verify();
        let functions = HashMap::from([("enum_identity".into(), callee.graph)]);
        for optimized in [false, true] {
            if optimized {
                assert_eq!(
                    fold_identity_calls(&mut caller.graph, &functions, None)
                        .unwrap()
                        .calls_folded,
                    1
                );
            }
            let result = execute(&caller.graph, &functions, &[]);
            if active_poison {
                assert!(matches!(result, Err(InterpretError::PoisonObserved(_))));
            } else {
                assert_eq!(
                    result.unwrap(),
                    SemanticOutcome::Return(Value::Variant {
                        variant: 0,
                        fields: vec![]
                    })
                );
            }
        }
    }
}

#[test]
fn an_actual_infinite_control_loop_is_not_an_identity() {
    let mut callee = Function::new("loop_forever", 1, false);
    let entry = callee
        .graph
        .add_node(Opcode::Jump, callee.control, &[], 0, ValueType::Control);
    let region =
        callee
            .graph
            .add_growable_node(Opcode::Region, NodeId::NONE, 2, 0, ValueType::Control);
    let back = callee
        .graph
        .add_node(Opcode::Jump, region, &[], 0, ValueType::Control);
    callee.graph.set_inputs(region, &[entry, back]);
    callee.verify();
    let mut caller = Function::new("caller", 1, false);
    let result = caller.call(&callee.graph, &caller.args.clone(), caller.int);
    caller.ret(result);
    caller.verify();
    let functions = HashMap::from([("loop_forever".into(), callee.graph)]);
    let before = caller.graph.dump();
    let stats = fold_identity_calls(&mut caller.graph, &functions, None).unwrap();
    assert_eq!(stats.calls_folded, 0);
    assert_eq!(stats.skipped[0].1, InlineSkip::Unsupported);
    assert_eq!(caller.graph.dump(), before);
}

#[test]
fn only_selected_calls_are_folded_and_repeated_seeds_are_deduplicated() {
    let mut callee = Function::new("identity", 1, false);
    callee.ret(callee.args[0]);
    callee.verify();
    let mut caller = Function::new("caller", 1, false);
    let first = caller.call(&callee.graph, &caller.args.clone(), caller.int);
    let second = caller.call(&callee.graph, &[first], caller.int);
    caller.ret(second);
    caller.verify();
    let source = caller.graph.input(second, 0);
    let functions = HashMap::from([("identity".into(), callee.graph)]);
    let stats = fold_identity_calls(
        &mut caller.graph,
        &functions,
        Some(&[source, source, second]),
    )
    .unwrap();
    assert_eq!(stats.calls_examined, 1);
    assert_eq!(stats.calls_folded, 1);
    assert_eq!(
        caller
            .graph
            .ids()
            .filter(|&n| caller.graph.op(n) == Opcode::Call)
            .count(),
        1
    );
    assert_eq!(
        execute(&caller.graph, &functions, &[Value::Scalar(42)]).unwrap(),
        SemanticOutcome::Return(Value::Scalar(42))
    );
}

#[test]
fn a_later_invalid_summary_rolls_back_the_whole_transaction() {
    let mut valid = Function::new("valid", 1, false);
    valid.ret(valid.args[0]);
    valid.verify();
    let mut invalid = Function::new("invalid", 1, false);
    invalid.ret(invalid.args[0]);
    invalid.verify();
    let mut caller = Function::new("caller", 1, false);
    let first = caller.call(&valid.graph, &caller.args.clone(), caller.int);
    let second = caller.call(&invalid.graph, &[first], caller.int);
    caller.ret(second);
    caller.verify();
    let boolean = invalid.graph.oracle.intern(TypeShape::Bool, None);
    invalid
        .graph
        .set_value_type(invalid.args[0], ValueType::Rust(boolean));
    let functions = HashMap::from([
        ("valid".into(), valid.graph),
        ("invalid".into(), invalid.graph),
    ]);
    let before = caller.graph.dump();
    assert!(matches!(
        fold_identity_calls(&mut caller.graph, &functions, None),
        Err(kfrc_son::InlineError::Verification(_))
    ));
    assert_eq!(caller.graph.dump(), before);
}

#[test]
fn ignored_abi_identity_keeps_its_source_value_and_requirement() {
    for enumeration in [false, true] {
        let mut callee = Function::new("ignored_identity", 1, false);
        let ty = if enumeration {
            let ty = callee.graph.oracle.intern(
                TypeShape::Adt {
                    name: "IgnoredEnum".into(),
                },
                Some(SizeAlign { size: 0, align: 1 }),
            );
            callee.graph.oracle.record_semantic_aggregate(
                ty,
                SemanticAggregate {
                    fields: vec![],
                    variants: vec![vec![]],
                },
            );
            callee.graph.oracle.record_semantic_enum(ty, vec![0]);
            callee
                .graph
                .oracle
                .record_semantic_inhabited_variants(ty, vec![0]);
            ty
        } else {
            callee
                .graph
                .oracle
                .intern(TypeShape::Unit, Some(SizeAlign { size: 0, align: 1 }))
        };
        callee.graph.abi.args[0] = PassMode::Ignore;
        callee.graph.abi.ret = PassMode::Ignore;
        callee.graph.set_value_type(
            callee.graph.start,
            ValueType::Tuple(vec![
                ValueType::Control,
                ValueType::Effect,
                ValueType::Rust(ty),
            ]),
        );
        callee
            .graph
            .set_value_type(callee.args[0], ValueType::Rust(ty));
        callee.ret(callee.args[0]);
        callee.verify();
        for undefined in [false, true] {
            let mut caller = Function::new("caller", 0, false);
            caller.graph.oracle = callee.graph.oracle.clone();
            caller.graph.abi.ret = PassMode::Ignore;
            let op = if undefined {
                Opcode::Undefined
            } else if enumeration {
                Opcode::MakeVariant
            } else {
                Opcode::Aggregate
            };
            let argument = caller.node(op, &[], 0, ty);
            let result = caller.call(&callee.graph, &[argument], ty);
            caller.ret(result);
            caller.verify();
            let functions = HashMap::from([("ignored_identity".into(), callee.graph.clone())]);
            let before = execute(&caller.graph, &functions, &[]);
            assert_eq!(
                fold_identity_calls(&mut caller.graph, &functions, None)
                    .unwrap()
                    .calls_folded,
                1
            );
            for node in caller
                .graph
                .ids()
                .filter(|&n| caller.graph.op(n) == Opcode::RequireDefined)
            {
                assert_eq!(
                    caller.graph.value_type(node),
                    &ValueType::Tuple(vec![ValueType::Control, ValueType::Effect])
                );
                assert!(caller.graph.find_projection(node, 2).is_none());
            }
            let after = execute(&caller.graph, &functions, &[]);
            // Unit has no poison leaves even when built from Undefined. An
            // enum still needs a known logical variant despite a zero-byte ABI.
            if undefined && enumeration {
                assert!(matches!(before, Err(InterpretError::PoisonObserved(_))));
                assert!(matches!(after, Err(InterpretError::PoisonObserved(_))));
            } else {
                let value = if enumeration {
                    Value::Variant {
                        variant: 0,
                        fields: vec![],
                    }
                } else {
                    Value::Aggregate(vec![])
                };
                assert_eq!(before, Ok(SemanticOutcome::Return(value)));
                assert_eq!(before, after);
            }
        }
    }
}

impl Function {
    fn new(name: &str, args: usize, shift_types: bool) -> Self {
        let mut types = RecordedTypes::new();
        if shift_types {
            types.intern(TypeShape::Unit, None);
            types.intern(TypeShape::Bool, Some(SizeAlign { size: 1, align: 1 }));
        }
        let int = types.intern(
            TypeShape::Int {
                width: IntegerWidth::W64,
                signed: false,
            },
            Some(SizeAlign { size: 8, align: 8 }),
        );
        let mut graph = FunctionGraph::new(
            name,
            FunctionAbi {
                args: vec![PassMode::Direct; args],
                ret: PassMode::Direct,
                c_variadic: false,
            },
            types,
        );
        graph.level = GraphLevel::Specialized;
        let mut outputs = vec![ValueType::Control, ValueType::Effect];
        outputs.extend(std::iter::repeat_n(ValueType::Rust(int), args));
        let start = graph.add_node(
            Opcode::Start,
            NodeId::NONE,
            &[],
            0,
            ValueType::Tuple(outputs),
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
        let args = (0..args)
            .map(|i| {
                graph.add_node(
                    Opcode::Projection,
                    NodeId::NONE,
                    &[start],
                    i as u32 + 2,
                    ValueType::Rust(int),
                )
            })
            .collect();
        Self {
            graph,
            int,
            control,
            effect,
            args,
        }
    }
    fn node(&mut self, op: Opcode, args: &[NodeId], payload: u32, ty: TyId) -> NodeId {
        self.graph
            .add_node(op, NodeId::NONE, args, payload, ValueType::Rust(ty))
    }
    fn ret(&mut self, value: NodeId) {
        self.graph.add_node(
            Opcode::Return,
            self.control,
            &[self.effect, value],
            0,
            ValueType::Control,
        );
    }
    fn call(&mut self, target: &FunctionGraph, args: &[NodeId], result: TyId) -> NodeId {
        let target = self.graph.add_callee(CallTarget {
            symbol: target.symbol.clone(),
            abi: target.abi.clone(),
        });
        let mut inputs = vec![self.effect];
        inputs.extend_from_slice(args);
        let call = self.graph.add_node(
            Opcode::Call,
            self.control,
            &inputs,
            target,
            ValueType::Tuple(vec![
                ValueType::Control,
                ValueType::Effect,
                ValueType::Rust(result),
            ]),
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
        self.node(Opcode::Projection, &[call], 2, result)
    }
    fn verify(&mut self) {
        verify_graph(&mut self.graph).unwrap();
    }
}
