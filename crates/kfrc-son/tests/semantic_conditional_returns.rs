//! Differential execution of a guarded bypass and its original call boundary.
use kfrc_body::{
    AggregateLayout, FieldLayout, FunctionAbi, IntegerWidth, Mutability, PassMode, RecordedTypes,
    SemanticAggregate, SizeAlign, TyId, TypeShape,
};
use kfrc_son::{
    CallTarget, ComparePredicate, ConditionalReturns, FunctionGraph, GraphLevel, InterpretError,
    Interpreter, NodeId, Opcode, SemanticOutcome, Value, ValueType, verify_graph,
};
use std::collections::{BTreeMap, BTreeSet};

#[path = "semantic_conditional_returns/transmutes.rs"]
mod transmutes;

#[derive(Clone, Copy)]
enum Argument {
    Word,
    Pair,
    Nested,
    Slice,
}

struct Function {
    graph: FunctionGraph,
    word: TyId,
    boolean: TyId,
    pointer: TyId,
    pair: TyId,
    nested: TyId,
    slice: TyId,
    args: Vec<NodeId>,
    control: NodeId,
    effect: NodeId,
}

impl Function {
    fn new(name: &str, parameters: &[Argument], shifted: bool) -> Self {
        let mut oracle = RecordedTypes::new();
        if shifted {
            oracle.intern(TypeShape::Unit, Some(SizeAlign { size: 0, align: 1 }));
        }
        let word = oracle.intern(
            TypeShape::Int {
                width: IntegerWidth::Pointer,
                signed: false,
            },
            Some(SizeAlign { size: 8, align: 8 }),
        );
        let boolean = oracle.intern(TypeShape::Bool, Some(SizeAlign { size: 1, align: 1 }));
        let byte = oracle.intern(
            TypeShape::Int {
                width: IntegerWidth::W8,
                signed: false,
            },
            Some(SizeAlign { size: 1, align: 1 }),
        );
        let pointer = oracle.intern(
            TypeShape::RawPointer {
                mutability: Mutability::Not,
                pointee: byte,
            },
            Some(SizeAlign { size: 8, align: 8 }),
        );
        let bytes = oracle.intern(TypeShape::Slice { element: byte }, None);
        let slice = oracle.intern(
            TypeShape::RawPointer {
                mutability: Mutability::Not,
                pointee: bytes,
            },
            Some(SizeAlign { size: 16, align: 8 }),
        );
        oracle.record_aggregate(
            slice,
            AggregateLayout {
                fields: vec![
                    FieldLayout {
                        ty: pointer,
                        offset: 0,
                    },
                    FieldLayout {
                        ty: word,
                        offset: 8,
                    },
                ],
                variants: vec![],
            },
        );
        let pair = oracle.intern(TypeShape::Tuple(vec![word, word]), None);
        let nested = oracle.intern(TypeShape::Tuple(vec![pair, word]), None);
        let parameter_types: Vec<_> = parameters
            .iter()
            .map(|kind| match kind {
                Argument::Word => word,
                Argument::Pair => pair,
                Argument::Nested => nested,
                Argument::Slice => slice,
            })
            .collect();
        let mut graph = FunctionGraph::new(
            name,
            FunctionAbi {
                args: vec![PassMode::Direct; parameters.len()],
                ret: PassMode::Direct,
                c_variadic: false,
            },
            oracle,
        );
        graph.level = GraphLevel::Specialized;
        let mut outputs = vec![ValueType::Control, ValueType::Effect];
        outputs.extend(parameter_types.iter().copied().map(ValueType::Rust));
        graph.start = graph.add_node(
            Opcode::Start,
            NodeId::NONE,
            &[],
            0,
            ValueType::Tuple(outputs),
        );
        let start = graph.start;
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
        let args = parameter_types
            .into_iter()
            .enumerate()
            .map(|(index, ty)| {
                graph.add_node(
                    Opcode::Projection,
                    NodeId::NONE,
                    &[start],
                    index as u32 + 2,
                    ValueType::Rust(ty),
                )
            })
            .collect();
        Self {
            graph,
            word,
            boolean,
            pointer,
            pair,
            nested,
            slice,
            args,
            control,
            effect,
        }
    }

    fn node(&mut self, op: Opcode, inputs: &[NodeId], payload: u32, ty: TyId) -> NodeId {
        self.graph
            .add_node(op, NodeId::NONE, inputs, payload, ValueType::Rust(ty))
    }

    fn project(&mut self, source: NodeId, slot: u32, ty: ValueType) -> NodeId {
        self.graph
            .add_node(Opcode::Projection, NodeId::NONE, &[source], slot, ty)
    }

    fn selector(&mut self, mut value: NodeId, kind: Argument) -> NodeId {
        if matches!(kind, Argument::Nested) {
            value = self.node(Opcode::Field, &[value], 0, self.pair);
        }
        if !matches!(kind, Argument::Word) {
            value = self.node(Opcode::Field, &[value], 1, self.word);
        }
        value
    }

    fn literal(&mut self, kind: Argument, length: u128, poison_sibling: bool) -> NodeId {
        let length = self.graph.add_constant(length, self.word);
        if matches!(kind, Argument::Word) {
            return length;
        }
        let sibling_ty = if matches!(kind, Argument::Slice) {
            self.pointer
        } else {
            self.word
        };
        let sibling = if poison_sibling {
            self.node(Opcode::Undefined, &[], 0, sibling_ty)
        } else {
            self.graph.add_constant(0, sibling_ty)
        };
        let mut value = if matches!(kind, Argument::Slice) {
            self.node(Opcode::WidePtr, &[sibling, length], 0, self.slice)
        } else {
            self.node(Opcode::Aggregate, &[sibling, length], 0, self.pair)
        };
        if matches!(kind, Argument::Nested) {
            value = self.node(Opcode::Aggregate, &[value, sibling], 0, self.nested);
        }
        value
    }

    fn call(&mut self, target: &FunctionGraph, args: &[NodeId]) -> NodeId {
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
                ValueType::Rust(self.boolean),
            ]),
        );
        self.control = self.project(call, 0, ValueType::Control);
        self.effect = self.project(call, 1, ValueType::Effect);
        self.project(call, 2, ValueType::Rust(self.boolean))
    }

    fn marker(&mut self, name: &str) -> NodeId {
        let declaration = Function::new(name, &[], false);
        self.call(&declaration.graph, &[])
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

    fn verify(&mut self) {
        verify_graph(&mut self.graph).unwrap();
    }
}

fn equality(kind: Argument, predicate: ComparePredicate, negate: bool, shared: bool) -> Function {
    let mut f = Function::new("arbitrary_relation", &[kind, kind, Argument::Word], true);
    let left = f.selector(f.args[0], kind);
    let right = f.selector(f.args[1], kind);
    let mut comparison = f.node(Opcode::Compare, &[left, right], predicate as u32, f.boolean);
    if negate {
        comparison = f.node(Opcode::BitNot, &[comparison], 0, f.boolean);
    }
    let branch = f.graph.add_node(
        Opcode::If,
        f.control,
        &[comparison],
        0,
        ValueType::Tuple(vec![ValueType::Control; 2]),
    );
    let yes = f.project(branch, 0, ValueType::Control);
    let no = f.project(branch, 1, ValueType::Control);
    let equal_is_yes = (predicate == ComparePredicate::Equal) ^ negate;
    let (equal, unequal) = if equal_is_yes { (yes, no) } else { (no, yes) };
    let entry_effect = f.effect;
    f.control = equal;
    let checked = f.marker("checked_payload");
    if shared {
        let equal_jump = f
            .graph
            .add_node(Opcode::Jump, f.control, &[], 0, ValueType::Control);
        let unequal_jump = f
            .graph
            .add_node(Opcode::Jump, unequal, &[], 0, ValueType::Control);
        let region = f.graph.add_node(
            Opcode::Region,
            NodeId::NONE,
            &[equal_jump, unequal_jump],
            0,
            ValueType::Control,
        );
        let zero = f.graph.add_constant(0, f.boolean);
        let value = f.graph.add_node(
            Opcode::Phi,
            region,
            &[region, checked, zero],
            0,
            ValueType::Rust(f.boolean),
        );
        f.effect = f.graph.add_node(
            Opcode::Phi,
            region,
            &[region, f.effect, entry_effect],
            0,
            ValueType::Effect,
        );
        f.control = region;
        f.ret(value);
    } else {
        f.ret(checked);
        f.control = unequal;
        f.effect = entry_effect;
        let zero = f.graph.add_constant(0, f.boolean);
        f.ret(zero);
    }
    f.verify();
    f
}

fn caller(callee: &FunctionGraph, kind: Argument, literal: bool) -> Function {
    let parameters = if literal {
        vec![kind]
    } else {
        vec![kind, kind]
    };
    let mut f = Function::new("caller", &parameters, false);
    f.marker("before");
    let rhs = if literal {
        f.literal(kind, 5, false)
    } else {
        f.args[1]
    };
    let unused = f.graph.add_constant(17, f.word);
    let value = f.call(callee, &[f.args[0], rhs, unused]);
    f.marker("after");
    f.ret(value);
    f.verify();
    f
}

fn table(callee: FunctionGraph) -> (BTreeMap<String, FunctionGraph>, ConditionalReturns) {
    let exact = BTreeSet::from([callee.symbol.clone()]);
    let bodies = BTreeMap::from([(callee.symbol.clone(), callee)]);
    let summaries = ConditionalReturns::infer_exact(&bodies, &exact);
    (bodies, summaries)
}

fn execute(
    graph: &FunctionGraph,
    bodies: &BTreeMap<String, FunctionGraph>,
    args: &[Value],
    events: &mut Vec<String>,
) -> Result<SemanticOutcome, InterpretError> {
    Interpreter::new(graph).run_values(args, &mut |symbol: &str, args: &[Value]| {
        if let Some(callee) = bodies.get(symbol) {
            return match execute(callee, bodies, args, events)? {
                SemanticOutcome::Return(value) => Ok(value),
                SemanticOutcome::Unreachable => {
                    Err(InterpretError::UndefinedBehaviour("unreachable".into()))
                }
            };
        }
        events.push(symbol.to_owned());
        Ok(Value::Scalar(1))
    })
}

fn argument(kind: Argument, length: u64) -> Value {
    let pair = Value::Aggregate(vec![Value::Scalar(0), Value::Scalar(length)]);
    match kind {
        Argument::Word => Value::Scalar(length),
        Argument::Pair => pair,
        Argument::Nested => Value::Aggregate(vec![pair, Value::Scalar(0)]),
        Argument::Slice => Value::WidePointer {
            data: Box::new(Value::Scalar(0)),
            metadata: Box::new(Value::Scalar(length)),
        },
    }
}

#[test]
fn exact_integer_and_nested_field_relations_keep_equal_calls_and_shared_phi_slots() {
    for kind in [
        Argument::Word,
        Argument::Pair,
        Argument::Nested,
        Argument::Slice,
    ] {
        for (predicate, negate, shared) in [
            (ComparePredicate::Equal, false, false),
            (ComparePredicate::NotEqual, false, true),
            (ComparePredicate::NotEqual, true, true),
        ] {
            let callee = equality(kind, predicate, negate, shared);
            let mut caller = caller(&callee.graph, kind, true);
            let before = caller.graph.clone();
            let (bodies, summaries) = table(callee.graph);
            assert!(summaries.has_candidates(&caller.graph));
            let stats = summaries.fold_calls(&mut caller.graph).unwrap();
            assert_eq!(stats.calls_split, 1);
            caller.verify();
            assert_eq!(
                caller
                    .graph
                    .ids()
                    .filter(|&n| caller.graph.op(n) == Opcode::Call)
                    .count(),
                3
            );
            assert_eq!(
                caller
                    .graph
                    .ids()
                    .filter(|&n| caller.graph.op(n) == Opcode::RequireDefined)
                    .count(),
                3
            );
            for length in [0, 1, 5, u64::MAX] {
                let args = [argument(kind, length)];
                let mut old_events = Vec::new();
                let mut new_events = Vec::new();
                let old = execute(&before, &bodies, &args, &mut old_events);
                let new = execute(&caller.graph, &bodies, &args, &mut new_events);
                assert_eq!(old, new);
                assert_eq!(old_events, new_events);
                assert_eq!(new_events.contains(&"checked_payload".into()), length == 5);
            }
        }
    }
}

#[test]
fn mismatch_still_observes_poison_in_data_metadata_unused_arguments_and_siblings() {
    for kind in [Argument::Pair, Argument::Slice] {
        for poisoned in [0, 1, 2] {
            let callee = equality(kind, ComparePredicate::Equal, false, true);
            let mut caller = Function::new("poison_caller", &[], false);
            caller.marker("before");
            let lhs = caller.literal(kind, 3, poisoned == 0);
            let rhs = caller.literal(kind, 5, false);
            if poisoned == 1 {
                let poison = caller.node(Opcode::Undefined, &[], 0, caller.word);
                caller.graph.set_input(lhs, 1, poison);
            }
            let unused = if poisoned == 2 {
                caller.node(Opcode::Undefined, &[], 0, caller.word)
            } else {
                caller.graph.add_constant(17, caller.word)
            };
            caller.call(&callee.graph, &[lhs, rhs, unused]);
            caller.marker("after");
            let zero = caller.graph.add_constant(0, caller.boolean);
            caller.ret(zero); // The conditional result is deliberately unused.
            caller.verify();
            let (bodies, summaries) = table(callee.graph);
            let before = caller.graph.clone();
            assert_eq!(
                summaries.fold_calls(&mut caller.graph).unwrap().calls_split,
                1
            );
            caller.verify();
            for graph in [&before, &caller.graph] {
                let mut events = Vec::new();
                assert!(matches!(
                    execute(graph, &bodies, &[], &mut events),
                    Err(InterpretError::PoisonObserved(_))
                ));
                assert_eq!(events, ["before"]);
            }
        }
    }
}

#[test]
fn nonexact_abi_mismatches_and_dynamic_comparisons_leave_the_call_unchanged() {
    for reason in ["nonexact", "abi", "dynamic"] {
        let callee = equality(Argument::Pair, ComparePredicate::Equal, false, true);
        let mut caller = caller(&callee.graph, Argument::Pair, reason != "dynamic");
        let mut exact = BTreeSet::from([callee.graph.symbol.clone()]);
        let mut source = callee.graph;
        if reason == "nonexact" {
            exact.clear();
        }
        if reason == "abi" {
            source.abi.args[0] = PassMode::Pair;
        }
        let bodies = BTreeMap::from([(source.symbol.clone(), source)]);
        let summaries = ConditionalReturns::infer_exact(&bodies, &exact);
        let before = caller.graph.dump();
        assert_eq!(
            summaries.fold_calls(&mut caller.graph).unwrap().calls_split,
            0
        );
        assert_eq!(caller.graph.dump(), before);
    }
}

#[test]
fn path_observations_reads_and_unused_anchored_division_prevent_a_summary() {
    for insertion in [
        "prefix-call",
        "mismatch-observe",
        "anchored-div",
        "mismatch-call",
        "mismatch-load",
    ] {
        let mut callee = equality(Argument::Pair, ComparePredicate::Equal, false, false);
        let guard = callee
            .graph
            .ids()
            .find(|&n| callee.graph.op(n) == Opcode::If)
            .unwrap();
        let mismatch = callee.graph.find_projection(guard, 1).unwrap();
        let ret = callee
            .graph
            .ids()
            .find(|&n| callee.graph.op(n) == Opcode::Return && callee.graph.control(n) == mismatch)
            .unwrap();
        let entry_effect = callee.graph.find_projection(callee.graph.start, 1).unwrap();
        if insertion == "anchored-div" {
            let zero = callee.graph.add_constant(0, callee.word);
            callee.graph.add_node(
                Opcode::DivUnsigned,
                mismatch,
                &[zero, zero],
                0,
                ValueType::Rust(callee.word),
            );
        } else if insertion == "mismatch-observe" {
            let observe = callee.graph.add_node(
                Opcode::RequireDefined,
                mismatch,
                &[entry_effect, callee.args[0]],
                0,
                ValueType::Tuple(vec![ValueType::Control, ValueType::Effect]),
            );
            let control = callee.project(observe, 0, ValueType::Control);
            let effect = callee.project(observe, 1, ValueType::Effect);
            callee.graph.set_control(ret, control);
            callee.graph.set_input(ret, 0, effect);
        } else if insertion == "mismatch-load" {
            let address = callee.graph.add_constant(0, callee.pointer);
            let byte = match callee.graph.oracle.table.shape(callee.pointer) {
                TypeShape::RawPointer { pointee, .. } => *pointee,
                _ => unreachable!(),
            };
            let load = callee.graph.add_node(
                Opcode::LoadValue,
                mismatch,
                &[entry_effect, address],
                0,
                ValueType::Tuple(vec![
                    ValueType::Control,
                    ValueType::Effect,
                    ValueType::Rust(byte),
                ]),
            );
            let control = callee.project(load, 0, ValueType::Control);
            let effect = callee.project(load, 1, ValueType::Effect);
            callee.graph.set_control(ret, control);
            callee.graph.set_input(ret, 0, effect);
        } else {
            let prefix = insertion == "prefix-call";
            callee.control = if prefix {
                callee.graph.control(guard)
            } else {
                mismatch
            };
            callee.effect = entry_effect;
            callee.marker("extra_observation");
            callee
                .graph
                .set_control(if prefix { guard } else { ret }, callee.control);
            if !prefix {
                callee.graph.set_input(ret, 0, callee.effect);
            } else {
                // Both eventual returns must carry the new prefix effect.
                callee.graph.set_input(ret, 0, callee.effect);
                let equal_call = callee
                    .graph
                    .ids()
                    .find(|&n| {
                        callee.graph.op(n) == Opcode::Call
                            && callee.graph.callees[callee.graph.payload(n) as usize].symbol
                                == "checked_payload"
                    })
                    .unwrap();
                callee.graph.set_input(equal_call, 0, callee.effect);
            }
        }
        callee.verify();
        let mut caller = caller(&callee.graph, Argument::Pair, true);
        let (_, summaries) = table(callee.graph);
        let before = caller.graph.dump();
        assert_eq!(
            summaries.fold_calls(&mut caller.graph).unwrap().calls_split,
            0,
            "{insertion}"
        );
        assert_eq!(caller.graph.dump(), before);
    }
}

#[test]
fn indirect_calls_and_missing_source_metadata_are_not_inferred_from_the_abi() {
    let callee = equality(Argument::Slice, ComparePredicate::Equal, false, true);
    let mut caller = caller(&callee.graph, Argument::Slice, true);
    let (_, summaries) = table(callee.graph.clone());
    let call = caller
        .graph
        .ids()
        .find(|&n| {
            caller.graph.op(n) == Opcode::Call
                && caller.graph.callees[caller.graph.payload(n) as usize].symbol
                    == callee.graph.symbol
        })
        .unwrap();
    let mut inputs = caller.graph.inputs(call).to_vec();
    let function = caller.graph.add_constant(0, caller.pointer);
    inputs.insert(1, function);
    let abi = caller.graph.call_abis.len() as u32;
    caller.graph.call_abis.push(callee.graph.abi.clone());
    caller
        .graph
        .set_opcode_payload(call, Opcode::CallIndirect, abi);
    caller.graph.set_inputs(call, &inputs);
    caller.verify();
    let before = caller.graph.dump();
    assert_eq!(
        summaries.fold_calls(&mut caller.graph).unwrap().calls_split,
        0
    );
    assert_eq!(caller.graph.dump(), before);

    // A wide ABI lane alone is not a logical metadata-field contract. This
    // deliberately incomplete source must fail inference, even with Field[1]
    // nodes and the same two-word size still present in the saved graph.
    let mut unknown = callee.graph;
    unknown.oracle.record_semantic_aggregate(
        callee.slice,
        SemanticAggregate {
            fields: vec![callee.pointer],
            variants: vec![],
        },
    );
    let mut caller = caller_for_unknown(&unknown);
    let (_, summaries) = table(unknown);
    let before = caller.graph.dump();
    assert!(!summaries.has_candidates(&caller.graph));
    assert_eq!(
        summaries.fold_calls(&mut caller.graph).unwrap().calls_split,
        0
    );
    assert_eq!(caller.graph.dump(), before);
}

fn caller_for_unknown(source: &FunctionGraph) -> Function {
    caller(source, Argument::Slice, true)
}

#[test]
fn one_wave_splits_only_the_original_calls_and_obeys_the_per_caller_limit() {
    let callee = equality(Argument::Word, ComparePredicate::Equal, false, false);
    let mut caller = Function::new("many_calls", &[Argument::Word], false);
    let rhs = caller.graph.add_constant(5, caller.word);
    let unused = caller.graph.add_constant(17, caller.word);
    let mut value = caller.graph.add_constant(0, caller.boolean);
    for _ in 0..70 {
        value = caller.call(&callee.graph, &[caller.args[0], rhs, unused]);
    }
    caller.ret(value);
    caller.verify();
    let (_, summaries) = table(callee.graph);
    let stats = summaries.fold_calls(&mut caller.graph).unwrap();
    assert_eq!(stats.calls_examined, 70);
    assert_eq!(stats.calls_split, 64);
    assert_eq!(
        caller
            .graph
            .ids()
            .filter(|&n| caller.graph.op(n) == Opcode::Call)
            .count(),
        70
    );
    caller.verify();
}
