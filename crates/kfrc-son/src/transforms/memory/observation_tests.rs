use super::*;
use crate::{
    CallTarget, ComparePredicate, InterpretError, Interpreter, SemanticOutcome, Value, verify_graph,
};
use kfrc_body::{
    FunctionAbi, IntegerWidth, PassMode, RecordedTypes, SemanticAggregate, SizeAlign, TyId,
};

struct Fixture {
    graph: FunctionGraph,
    word: TyId,
    boolean: TyId,
    unit: TyId,
    choice: TyId,
    control: NodeId,
    effect: NodeId,
    arg: NodeId,
}
impl Fixture {
    fn new(enum_arg: bool) -> Self {
        let mut types = RecordedTypes::new();
        let word = types.intern(
            TypeShape::Int {
                width: IntegerWidth::W64,
                signed: false,
            },
            Some(SizeAlign { size: 8, align: 8 }),
        );
        let boolean = types.intern(TypeShape::Bool, Some(SizeAlign { size: 1, align: 1 }));
        let unit = types.intern(TypeShape::Unit, Some(SizeAlign { size: 0, align: 1 }));
        let choice = types.intern(
            TypeShape::Adt {
                name: "Choice".into(),
            },
            None,
        );
        types.record_semantic_aggregate(
            choice,
            SemanticAggregate {
                fields: vec![],
                variants: vec![vec![word], vec![word]],
            },
        );
        types.record_semantic_enum(choice, vec![0, 1]);
        let arg_ty = if enum_arg { choice } else { word };
        let mut graph = FunctionGraph::new(
            "observation",
            FunctionAbi {
                args: vec![PassMode::Direct],
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
            ValueType::Tuple(vec![
                ValueType::Control,
                ValueType::Effect,
                ValueType::Rust(arg_ty),
            ]),
        );
        graph.start = start;
        let control = projection(&mut graph, start, 0, ValueType::Control);
        let effect = projection(&mut graph, start, 1, ValueType::Effect);
        let arg = projection(&mut graph, start, 2, ValueType::Rust(arg_ty));
        Self {
            graph,
            word,
            boolean,
            unit,
            choice,
            control,
            effect,
            arg,
        }
    }
    fn fixed(&mut self, op: Opcode, args: &[NodeId], payload: u32, ty: TyId) -> NodeId {
        let mut inputs = vec![self.effect];
        inputs.extend_from_slice(args);
        let node = self.graph.add_node(
            op,
            self.control,
            &inputs,
            payload,
            ValueType::Tuple(vec![
                ValueType::Control,
                ValueType::Effect,
                ValueType::Rust(ty),
            ]),
        );
        self.control = projection(&mut self.graph, node, 0, ValueType::Control);
        self.effect = projection(&mut self.graph, node, 1, ValueType::Effect);
        projection(&mut self.graph, node, 2, ValueType::Rust(ty))
    }
    fn observe(&mut self, value: NodeId) {
        let node = self.graph.add_node(
            Opcode::RequireDefined,
            self.control,
            &[self.effect, value],
            0,
            ValueType::Tuple(vec![ValueType::Control, ValueType::Effect]),
        );
        self.control = projection(&mut self.graph, node, 0, ValueType::Control);
        self.effect = projection(&mut self.graph, node, 1, ValueType::Effect);
    }
    fn call(&mut self, indirect: bool, ignored: bool) -> NodeId {
        let abi = FunctionAbi {
            args: vec![],
            ret: if ignored {
                PassMode::Ignore
            } else {
                PassMode::Direct
            },
            c_variadic: false,
        };
        let ty = if ignored { self.unit } else { self.word };
        if indirect {
            let pointer_ty = self.graph.oracle.intern(
                TypeShape::RawPointer {
                    mutability: kfrc_body::Mutability::Not,
                    pointee: self.unit,
                },
                Some(SizeAlign { size: 8, align: 8 }),
            );
            let pointer = self
                .graph
                .add_constant(if ignored { 124 } else { 123 }, pointer_ty);
            let payload = self.graph.call_abis.len() as u32;
            self.graph.call_abis.push(abi);
            self.fixed(Opcode::CallIndirect, &[pointer], payload, ty)
        } else {
            let payload = self.graph.callees.len() as u32;
            self.graph.callees.push(CallTarget {
                symbol: if ignored { "tick_unit" } else { "tick" }.into(),
                abi,
            });
            self.fixed(Opcode::Call, &[], payload, ty)
        }
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
    fn poison(&mut self, ty: TyId) -> NodeId {
        self.graph
            .add_node(Opcode::Undefined, NodeId::NONE, &[], 0, ValueType::Rust(ty))
    }
    fn jump(&mut self) -> NodeId {
        self.graph
            .add_node(Opcode::Jump, self.control, &[], 0, ValueType::Control)
    }
    fn count(&self) -> usize {
        self.graph
            .ids()
            .filter(|&n| self.graph.op(n) == Opcode::RequireDefined)
            .count()
    }
    fn optimize(&mut self) -> ObservationStats {
        verify_graph(&mut self.graph).unwrap();
        let stats = run(&mut self.graph);
        verify_graph(&mut self.graph).unwrap();
        stats
    }
}
fn projection(graph: &mut FunctionGraph, source: NodeId, index: u32, ty: ValueType) -> NodeId {
    graph.add_node(Opcode::Projection, NodeId::NONE, &[source], index, ty)
}
struct Calls {
    trace: Vec<String>,
    poison: bool,
}
impl crate::SemanticCallHandler for Calls {
    fn call(&mut self, name: &str, _: &[Value]) -> Result<Value, InterpretError> {
        self.trace.push(name.into());
        Ok(if self.poison {
            Value::Poison
        } else if matches!(name, "tick_unit" | "indirect@124") {
            Value::Aggregate(vec![])
        } else {
            Value::Scalar(19)
        })
    }
    fn call_indirect(&mut self, address: u64, args: &[Value]) -> Result<Value, InterpretError> {
        self.call(&format!("indirect@{address}"), args)
    }
}
fn execute(
    graph: &FunctionGraph,
    argument: Value,
    poison_return: bool,
) -> (Result<SemanticOutcome, InterpretError>, Vec<String>) {
    let mut calls = Calls {
        trace: vec![],
        poison: poison_return,
    };
    let result = Interpreter::new(graph).run_values(&[argument], &mut calls);
    (result, calls.trace)
}

#[test]
fn repeated_identity_keeps_first_observation_and_intervening_call() {
    for poisoned in [false, true] {
        let mut f = Fixture::new(false);
        let source = if poisoned { f.poison(f.word) } else { f.arg };
        let opaque = f.fixed(Opcode::Opaque, &[source], 0, f.word);
        f.observe(opaque);
        f.call(false, false);
        f.observe(opaque);
        f.ret(opaque);
        let original = f.graph.clone();
        assert_eq!(f.optimize().observations_removed, 1);
        assert_eq!(f.count(), 1);
        let after = execute(&f.graph, Value::Scalar(42), false);
        assert_eq!(after, execute(&original, Value::Scalar(42), false));
        if poisoned {
            assert!(matches!(after.0, Err(InterpretError::PoisonObserved(_))));
            assert!(after.1.is_empty());
        } else {
            assert_eq!(after.0, Ok(SemanticOutcome::Return(Value::Scalar(42))));
            assert_eq!(after.1, ["tick"]);
        }
    }
}

#[test]
fn entry_and_source_call_results_are_defined_independently_of_abi() {
    let mut f = Fixture::new(false);
    f.observe(f.arg);
    f.ret(f.arg);
    let original = f.graph.clone();
    assert_eq!(f.optimize().observations_removed, 1);
    for value in [Value::Scalar(8), Value::Poison] {
        assert_eq!(
            execute(&f.graph, value.clone(), false),
            execute(&original, value, false)
        );
    }
    for indirect in [false, true] {
        for ignored in [false, true] {
            let mut f = Fixture::new(false);
            let returned = f.call(indirect, ignored);
            f.observe(returned);
            f.ret(f.arg);
            let original = f.graph.clone();
            assert_eq!(f.optimize().observations_removed, 1);
            assert_eq!(
                execute(&f.graph, Value::Scalar(8), true),
                execute(&original, Value::Scalar(8), true)
            );
            assert_eq!(
                execute(&f.graph, Value::Scalar(8), false),
                execute(&original, Value::Scalar(8), false)
            );
        }
    }
}

#[test]
fn an_observation_on_only_one_branch_does_not_prove_the_merge() {
    let mut f = Fixture::new(false);
    let poison = f.poison(f.word);
    let zero = f.graph.add_constant(0, f.word);
    let condition = f.graph.add_node(
        Opcode::Compare,
        NodeId::NONE,
        &[f.arg, zero],
        ComparePredicate::Equal as u32,
        ValueType::Rust(f.boolean),
    );
    let branch = f.graph.add_node(
        Opcode::If,
        f.control,
        &[condition],
        0,
        ValueType::Tuple(vec![ValueType::Control; 2]),
    );
    let left = projection(&mut f.graph, branch, 0, ValueType::Control);
    let right = projection(&mut f.graph, branch, 1, ValueType::Control);
    let initial_effect = f.effect;
    f.control = left;
    f.observe(poison);
    let observed_effect = f.effect;
    let left_jump = f.jump();
    f.control = right;
    let right_jump = f.jump();
    let region = f.graph.add_node(
        Opcode::Region,
        NodeId::NONE,
        &[left_jump, right_jump],
        0,
        ValueType::Control,
    );
    f.effect = f.graph.add_node(
        Opcode::Phi,
        NodeId::NONE,
        &[region, observed_effect, initial_effect],
        0,
        ValueType::Effect,
    );
    f.control = region;
    f.observe(poison);
    f.ret(f.arg);
    let original = f.graph.clone();
    assert_eq!(f.optimize().observations_removed, 0);
    for arg in [0, 1] {
        assert_eq!(
            execute(&f.graph, Value::Scalar(arg), false),
            execute(&original, Value::Scalar(arg), false)
        );
    }
}

#[test]
fn loop_phi_observation_does_not_reuse_a_previous_iterations_fact() {
    let mut f = Fixture::new(false);
    let one = f.graph.add_constant(1, f.word);
    let poison = f.poison(f.word);
    let entry = f.jump();
    let region = f
        .graph
        .add_growable_node(Opcode::Region, NodeId::NONE, 2, 0, ValueType::Control);
    f.graph.set_input(region, 0, entry);
    let value = f.graph.add_node(
        Opcode::Phi,
        NodeId::NONE,
        &[region, one, poison],
        0,
        ValueType::Rust(f.word),
    );
    let effects = f
        .graph
        .add_growable_node(Opcode::Phi, NodeId::NONE, 3, 0, ValueType::Effect);
    f.graph.set_input(effects, 0, region);
    f.graph.set_input(effects, 1, f.effect);
    f.control = region;
    f.effect = effects;
    f.observe(value);
    f.call(false, false);
    f.observe(value);
    let back = f.jump();
    f.graph.set_input(region, 1, back);
    f.graph.set_input(effects, 2, f.effect);
    let original = f.graph.clone();
    assert_eq!(f.optimize().observations_removed, 1);
    assert_eq!(f.count(), 1);
    let result = execute(&f.graph, Value::Scalar(0), false);
    assert_eq!(result, execute(&original, Value::Scalar(0), false));
    assert!(matches!(result.0, Err(InterpretError::PoisonObserved(_))));
    assert_eq!(result.1, ["tick"]);
}

#[test]
fn nested_constructor_phi_uses_proven_active_fields_on_each_incoming_edge() {
    for inactive in [false, true] {
        let mut f = Fixture::new(true);
        let outer = f.graph.oracle.intern(
            TypeShape::Adt {
                name: "Outer".into(),
            },
            None,
        );
        f.graph.oracle.record_semantic_aggregate(
            outer,
            SemanticAggregate {
                fields: vec![],
                variants: vec![vec![f.choice], vec![f.choice]],
            },
        );
        f.graph.oracle.record_semantic_enum(outer, vec![0, 1]);
        let cases = f.graph.variant_switches.len() as u32;
        f.graph.variant_switches.push(vec![0]);
        let branch = f.graph.add_node(
            Opcode::VariantSwitch,
            f.control,
            &[f.arg],
            cases,
            ValueType::Tuple(vec![ValueType::Control; 2]),
        );
        let mut edges = Vec::new();
        let mut incoming = Vec::new();
        for variant in 0..2 {
            let control = projection(&mut f.graph, branch, variant, ValueType::Control);
            let witness = f.graph.add_node(
                Opcode::VariantWitness,
                control,
                &[f.arg, control],
                variant,
                ValueType::Witness,
            );
            let selected = if inactive { 1 - variant } else { variant };
            let field_index = f.graph.intern_variant_field(selected, 0);
            let inputs = if inactive {
                vec![f.arg]
            } else {
                vec![f.arg, witness]
            };
            let field = f.graph.add_node(
                Opcode::VariantField,
                NodeId::NONE,
                &inputs,
                field_index,
                ValueType::Rust(f.word),
            );
            let inner = f.graph.add_node(
                Opcode::MakeVariant,
                NodeId::NONE,
                &[field],
                variant,
                ValueType::Rust(f.choice),
            );
            let value = f.graph.add_node(
                Opcode::MakeVariant,
                NodeId::NONE,
                &[inner],
                variant,
                ValueType::Rust(outer),
            );
            incoming.push(value);
            f.control = control;
            edges.push(f.jump());
            // A dead inactive projection stays a lazy poison value. Neither
            // analysis nor removal may turn it into an eager failure.
            let wrong = f.graph.intern_variant_field(1 - variant, 0);
            f.graph.add_node(
                Opcode::VariantField,
                NodeId::NONE,
                &[f.arg],
                wrong,
                ValueType::Rust(f.word),
            );
        }
        let region = f
            .graph
            .add_node(Opcode::Region, NodeId::NONE, &edges, 0, ValueType::Control);
        let phi = f.graph.add_node(
            Opcode::Phi,
            NodeId::NONE,
            &[region, incoming[0], incoming[1]],
            0,
            ValueType::Rust(outer),
        );
        f.control = region;
        f.observe(phi);
        f.ret(f.arg);
        let original = f.graph.clone();
        assert_eq!(f.optimize().observations_removed, usize::from(!inactive));
        for variant in 0..2 {
            let arg = Value::Variant {
                variant,
                fields: vec![Value::Scalar(37)],
            };
            let result = execute(&f.graph, arg.clone(), false);
            assert_eq!(result, execute(&original, arg.clone(), false));
            if inactive {
                assert!(matches!(result.0, Err(InterpretError::PoisonObserved(_))));
            } else {
                assert_eq!(result.0, Ok(SemanticOutcome::Return(arg)));
            }
        }
    }
}

#[test]
fn variant_witness_does_not_prove_poison_payload_defined() {
    let mut f = Fixture::new(false);
    let poison = f.poison(f.word);
    let value = f.graph.add_node(
        Opcode::MakeVariant,
        NodeId::NONE,
        &[poison],
        0,
        ValueType::Rust(f.choice),
    );
    let witness = f.graph.add_node(
        Opcode::VariantWitness,
        f.control,
        &[value, f.control],
        0,
        ValueType::Witness,
    );
    let payload = f.graph.intern_variant_field(0, 0);
    let field = f.graph.add_node(
        Opcode::VariantField,
        NodeId::NONE,
        &[value, witness],
        payload,
        ValueType::Rust(f.word),
    );
    f.observe(field);
    f.call(false, false);
    f.observe(field);
    f.ret(f.arg);
    let original = f.graph.clone();
    assert_eq!(f.optimize().observations_removed, 1);
    assert_eq!(f.count(), 1);
    let result = execute(&f.graph, Value::Scalar(42), false);
    assert_eq!(result, execute(&original, Value::Scalar(42), false));
    assert!(matches!(result.0, Err(InterpretError::PoisonObserved(_))));
    assert!(result.1.is_empty());
}

#[test]
fn assumed_fact_does_not_prove_first_observation_or_erase_its_fixed_ub() {
    let mut f = Fixture::new(false);
    let pointer_ty = f.graph.oracle.intern(
        TypeShape::RawPointer {
            mutability: kfrc_body::Mutability::Not,
            pointee: f.word,
        },
        Some(SizeAlign { size: 8, align: 8 }),
    );
    let null = f.graph.add_constant(0, pointer_ty);
    let nonnull = f.graph.add_node(
        Opcode::Compare,
        NodeId::NONE,
        &[null, null],
        ComparePredicate::NotEqual as u32,
        ValueType::Rust(f.boolean),
    );
    let assumption = f.graph.add_node(
        Opcode::Assume,
        f.control,
        &[f.effect, nonnull],
        0,
        ValueType::Tuple(vec![ValueType::Control, ValueType::Effect]),
    );
    f.control = projection(&mut f.graph, assumption, 0, ValueType::Control);
    f.effect = projection(&mut f.graph, assumption, 1, ValueType::Effect);
    let poison = f.poison(f.word);
    f.observe(poison);
    f.call(false, false);
    f.observe(poison);
    f.ret(f.arg);
    let original = f.graph.clone();
    assert_eq!(f.optimize().observations_removed, 1);
    assert_eq!(f.count(), 1);
    assert!(!f.graph.is_dead(assumption));
    let result = execute(&f.graph, Value::Scalar(42), false);
    assert_eq!(result, execute(&original, Value::Scalar(42), false));
    assert!(matches!(
        result.0,
        Err(InterpretError::UndefinedBehaviour(_))
    ));
    assert!(result.1.is_empty());
}

#[test]
fn pass_does_not_apply_s_entry_facts_to_r() {
    let mut f = Fixture::new(false);
    f.observe(f.arg);
    f.ret(f.arg);
    f.graph.level = GraphLevel::Rust;
    f.graph.rust_signature = Some(crate::RustSignature {
        args: vec![f.word],
        ret: f.word,
    });
    verify_graph(&mut f.graph).unwrap();
    assert_eq!(run(&mut f.graph), ObservationStats::default());
    assert_eq!(f.count(), 1);
}

#[test]
fn full_and_seam_pipelines_remove_duplicate_observations_and_preserve_call_trace() {
    for seams in [false, true] {
        let mut f = Fixture::new(false);
        let opaque = f.fixed(Opcode::Opaque, &[f.arg], 0, f.word);
        f.observe(opaque);
        f.call(false, false);
        f.observe(opaque);
        let observed = opaque;
        f.ret(observed);
        verify_graph(&mut f.graph).unwrap();
        let original = f.graph.clone();
        let stats = if seams {
            crate::optimize_specialized_seams(&mut f.graph, &[observed], &[]).unwrap()
        } else {
            crate::optimize_specialized_graph(&mut f.graph).unwrap()
        };
        assert_eq!(stats.observations_removed, 1);
        assert_eq!(f.count(), 1);
        assert!(stats.observation_nodes_indexed > 0);
        for value in [0, 42, u64::MAX] {
            assert_eq!(
                execute(&f.graph, Value::Scalar(value), false),
                execute(&original, Value::Scalar(value), false)
            );
        }
    }
}
