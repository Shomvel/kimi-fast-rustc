//! Requirements constrain R/S execution without creating another business value.
use kfrc_body::{FunctionAbi, IntegerWidth, PassMode, RecordedTypes, SizeAlign, TyId, TypeShape};
use kfrc_son::{
    FunctionGraph, GraphLevel, InterpretError, Interpreter, NodeId, Opcode, RustSignature,
    SemanticOutcome, Value, ValueType, verify_graph,
};

struct Fixture {
    graph: FunctionGraph,
    word: TyId,
    control: NodeId,
    effect: NodeId,
}
impl Fixture {
    fn new() -> Self {
        let mut types = RecordedTypes::new();
        let word = types.intern(
            TypeShape::Int {
                width: IntegerWidth::W64,
                signed: false,
            },
            Some(SizeAlign { size: 8, align: 8 }),
        );
        let mut graph = FunctionGraph::new(
            "requirements",
            FunctionAbi {
                args: vec![],
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
        Self {
            graph,
            word,
            control,
            effect,
        }
    }
    fn value(&mut self, opcode: Opcode, ty: TyId) -> NodeId {
        self.graph
            .add_node(opcode, NodeId::NONE, &[], 0, ValueType::Rust(ty))
    }
    fn require(&mut self, value: NodeId) -> NodeId {
        let node = self.graph.add_node(
            Opcode::RequireDefined,
            self.control,
            &[self.effect, value],
            0,
            ValueType::Tuple(vec![ValueType::Control, ValueType::Effect]),
        );
        self.control = self.graph.add_node(
            Opcode::Projection,
            NodeId::NONE,
            &[node],
            0,
            ValueType::Control,
        );
        self.effect = self.graph.add_node(
            Opcode::Projection,
            NodeId::NONE,
            &[node],
            1,
            ValueType::Effect,
        );
        node
    }
    fn ret(&mut self, value: Option<NodeId>) -> NodeId {
        let mut inputs = vec![self.effect];
        inputs.extend(value);
        self.graph
            .add_node(Opcode::Return, self.control, &inputs, 0, ValueType::Control)
    }
    fn run(&mut self) -> Result<SemanticOutcome, InterpretError> {
        verify_graph(&mut self.graph).unwrap();
        Interpreter::new(&self.graph).run_values(&[], &mut |_: &str, _: &[Value]| unreachable!())
    }
}

#[test]
fn requirement_preserves_original_value_identity_and_observes_unused_poison() {
    for poison in [false, true] {
        let mut f = Fixture::new();
        let value = if poison {
            f.value(Opcode::Undefined, f.word)
        } else {
            f.graph.add_constant(37, f.word)
        };
        let requirement = f.require(value);
        let zero = f.graph.add_constant(0, f.word);
        let result = if poison { zero } else { value };
        let ret = f.ret(Some(result));
        assert_eq!(
            f.graph.value_type(requirement),
            &ValueType::Tuple(vec![ValueType::Control, ValueType::Effect])
        );
        assert_eq!(f.graph.input(ret, 1), result);
        let outcome = f.run();
        if poison {
            assert!(matches!(outcome, Err(InterpretError::PoisonObserved(_))));
        } else {
            assert_eq!(outcome, Ok(SemanticOutcome::Return(Value::Scalar(37))));
        }
    }
}

#[test]
fn requirement_has_no_value_projection_and_rejects_old_tuple_contract() {
    for old_tuple in [false, true] {
        let mut f = Fixture::new();
        let value = f.graph.add_constant(7, f.word);
        let requirement = f.require(value);
        if old_tuple {
            f.graph.set_value_type(
                requirement,
                ValueType::Tuple(vec![
                    ValueType::Control,
                    ValueType::Effect,
                    ValueType::Rust(f.word),
                ]),
            );
        } else {
            f.graph.add_node(
                Opcode::Projection,
                NodeId::NONE,
                &[requirement],
                2,
                ValueType::Rust(f.word),
            );
        }
        f.ret(Some(value));
        assert!(verify_graph(&mut f.graph).is_err());
    }
}

#[test]
fn requirement_is_legal_in_r_and_s_but_never_in_published_l() {
    for level in [
        GraphLevel::Rust,
        GraphLevel::Specialized,
        GraphLevel::Layout,
    ] {
        let mut f = Fixture::new();
        let value = f.graph.add_constant(7, f.word);
        f.require(value);
        f.ret(Some(value));
        f.graph.level = level;
        if level == GraphLevel::Rust {
            f.graph.rust_signature = Some(RustSignature {
                args: vec![],
                ret: f.word,
            });
        }
        assert_eq!(
            verify_graph(&mut f.graph).is_ok(),
            level != GraphLevel::Layout
        );
    }
}

#[test]
fn requirements_need_effect_rust_value_control_and_zero_payload() {
    for bad in 0..4 {
        let mut f = Fixture::new();
        let value = f.graph.add_constant(7, f.word);
        let requirement = f.require(value);
        f.ret(Some(value));
        match bad {
            0 => f.graph.set_input(requirement, 0, value),
            1 => f.graph.set_input(requirement, 1, f.graph.start),
            2 => f.graph.set_control(requirement, NodeId::NONE),
            _ => f.graph.set_payload(requirement, 1),
        }
        assert!(verify_graph(&mut f.graph).is_err(), "bad requirement {bad}");
    }
}

#[test]
fn maybe_uninit_bytes_and_empty_arrays_do_not_become_poison_requirements() {
    for container in ["mu", "empty"] {
        let mut f = Fixture::new();
        let ty = if container == "mu" {
            let mu = f.graph.oracle.intern(
                TypeShape::Adt {
                    name: "MarkedUninit".into(),
                },
                Some(SizeAlign { size: 8, align: 8 }),
            );
            f.graph.oracle.record_maybe_uninit(mu, f.word);
            mu
        } else {
            f.graph.oracle.intern(
                TypeShape::Array {
                    element: f.word,
                    length: 0,
                },
                Some(SizeAlign { size: 0, align: 8 }),
            )
        };
        let uninit = f.value(Opcode::Undefined, ty);
        f.require(uninit);
        let seven = f.graph.add_constant(7, f.word);
        f.ret(Some(seven));
        assert_eq!(f.run(), Ok(SemanticOutcome::Return(Value::Scalar(7))));
    }
}

#[test]
fn ignore_transport_does_not_remove_source_return_values() {
    for level in [
        GraphLevel::Rust,
        GraphLevel::Specialized,
        GraphLevel::Layout,
    ] {
        for has_value in [false, true] {
            let mut f = Fixture::new();
            f.graph.abi.ret = PassMode::Ignore;
            f.graph.level = level;
            let unit = f
                .graph
                .oracle
                .intern(TypeShape::Unit, Some(SizeAlign { size: 0, align: 1 }));
            if level == GraphLevel::Rust {
                f.graph.rust_signature = Some(RustSignature {
                    args: vec![],
                    ret: unit,
                });
            }
            let unit_value = f.graph.add_constant(0, unit);
            f.ret(has_value.then_some(unit_value));
            assert_eq!(
                verify_graph(&mut f.graph).is_ok(),
                has_value == (level != GraphLevel::Layout)
            );
        }
    }
    let mut f = Fixture::new();
    f.graph.abi.ret = PassMode::Ignore;
    let poison = f.value(Opcode::Undefined, f.word);
    f.ret(Some(poison));
    assert!(matches!(f.run(), Err(InterpretError::PoisonObserved(_))));
}
