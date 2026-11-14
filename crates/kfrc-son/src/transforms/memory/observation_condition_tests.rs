use super::*;
use crate::{
    ArithmeticFlags, CallTarget, ComparePredicate as P, InterpretError, Interpreter,
    SemanticCallHandler, SemanticMemory, SemanticOutcome, Value, verify_graph,
};
use kfrc_body::{FunctionAbi, IntegerWidth, Mutability, PassMode, RecordedTypes, SizeAlign, TyId};

struct Fixture {
    graph: FunctionGraph,
    byte: TyId,
    word: TyId,
    boolean: TyId,
    pointer: TyId,
    input: NodeId,
    arg: NodeId,
    control: NodeId,
    effect: NodeId,
}

impl Fixture {
    fn new() -> Self {
        let mut types = RecordedTypes::new();
        let byte = types.intern(
            TypeShape::Int {
                width: IntegerWidth::W8,
                signed: false,
            },
            Some(SizeAlign { size: 1, align: 1 }),
        );
        let word = types.intern(
            TypeShape::Int {
                width: IntegerWidth::Pointer,
                signed: false,
            },
            Some(SizeAlign { size: 8, align: 8 }),
        );
        let boolean = types.intern(TypeShape::Bool, Some(SizeAlign { size: 1, align: 1 }));
        let pointer = types.intern(
            TypeShape::RawPointer {
                mutability: Mutability::Mut,
                pointee: byte,
            },
            Some(SizeAlign { size: 8, align: 8 }),
        );
        let mut graph = FunctionGraph::new(
            "observed_table_checks",
            FunctionAbi {
                args: vec![PassMode::Direct; 2],
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
                ValueType::Rust(pointer),
                ValueType::Rust(word),
            ]),
        );
        graph.start = start;
        let control = projection(&mut graph, start, 0, ValueType::Control);
        let effect = projection(&mut graph, start, 1, ValueType::Effect);
        let input = projection(&mut graph, start, 2, ValueType::Rust(pointer));
        let arg = projection(&mut graph, start, 3, ValueType::Rust(word));
        Self {
            graph,
            byte,
            word,
            boolean,
            pointer,
            input,
            arg,
            control,
            effect,
        }
    }
    fn value(&mut self, op: Opcode, inputs: &[NodeId], payload: u32, ty: TyId) -> NodeId {
        self.graph
            .add_node(op, NodeId::NONE, inputs, payload, ValueType::Rust(ty))
    }
    fn fixed(
        &mut self,
        op: Opcode,
        inputs: &[NodeId],
        payload: u32,
        ty: Option<TyId>,
    ) -> Option<NodeId> {
        let mut operands = vec![self.effect];
        operands.extend_from_slice(inputs);
        let mut outputs = vec![ValueType::Control, ValueType::Effect];
        outputs.extend(ty.map(ValueType::Rust));
        let node = self.graph.add_node(
            op,
            self.control,
            &operands,
            payload,
            ValueType::Tuple(outputs),
        );
        self.control = projection(&mut self.graph, node, 0, ValueType::Control);
        self.effect = projection(&mut self.graph, node, 1, ValueType::Effect);
        ty.map(|ty| projection(&mut self.graph, node, 2, ValueType::Rust(ty)))
    }
    fn load(&mut self, address: NodeId) -> NodeId {
        self.fixed(Opcode::LoadValue, &[address], 0, Some(self.byte))
            .unwrap()
    }
    fn observe(&mut self, value: NodeId) {
        self.fixed(Opcode::RequireDefined, &[value], 0, None);
    }
    fn widen(&mut self, byte: NodeId) -> NodeId {
        self.value(Opcode::IntCast, &[byte], 0, self.word)
    }
    fn condition(&mut self, value: NodeId) -> NodeId {
        let wide = self.widen(value);
        let limit = self.graph.add_constant(256, self.word);
        self.value(
            Opcode::Compare,
            &[wide, limit],
            P::UnsignedLess as u32,
            self.boolean,
        )
    }
    fn branch(&mut self, condition: NodeId) -> (NodeId, [NodeId; 2]) {
        let branch = self.graph.add_node(
            Opcode::If,
            self.control,
            &[condition],
            0,
            ValueType::Tuple(vec![ValueType::Control; 2]),
        );
        let edges =
            [0, 1].map(|slot| projection(&mut self.graph, branch, slot, ValueType::Control));
        (branch, edges)
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
    fn jump(&mut self) -> NodeId {
        self.graph
            .add_node(Opcode::Jump, self.control, &[], 0, ValueType::Control)
    }
    fn call(&mut self, name: &str) {
        let target = self.graph.add_callee(CallTarget {
            symbol: name.into(),
            abi: FunctionAbi {
                args: vec![PassMode::Direct],
                ret: PassMode::Direct,
                c_variadic: false,
            },
        });
        self.fixed(Opcode::Call, &[self.input], target, Some(self.word));
    }
    fn bounds_check(&mut self, condition: NodeId) -> NodeId {
        let (branch, [yes, no]) = self.branch(condition);
        let effect = self.effect;
        self.control = no;
        self.call("bounds_panic");
        self.ret(self.arg);
        self.control = yes;
        self.effect = effect;
        branch
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

#[derive(Default)]
struct Calls(Vec<String>);
impl SemanticCallHandler for Calls {
    fn call(&mut self, name: &str, _: &[Value]) -> Result<Value, InterpretError> {
        self.0.push(name.into());
        Ok(Value::Scalar(0))
    }
    fn call_with_memory(
        &mut self,
        name: &str,
        args: &[Value],
        memory: &mut SemanticMemory,
    ) -> Result<Value, InterpretError> {
        if name == "invalidate" {
            let Value::Scalar(address) = args[0] else {
                unreachable!()
            };
            memory.write_bytes(address, &[None])?;
        }
        self.call(name, args)
    }
}

fn execute(
    graph: &FunctionGraph,
    bytes: [Option<u8>; 8],
    arg: u64,
) -> (
    Result<SemanticOutcome, InterpretError>,
    Vec<String>,
    Vec<Option<u8>>,
) {
    let mut memory = SemanticMemory::new();
    let input = memory.allocate(8, 1, true).unwrap();
    memory.write_bytes(input, &bytes).unwrap();
    let mut calls = Calls::default();
    let result = Interpreter::new(graph).run_values_in_memory(
        &[Value::Scalar(input), Value::Scalar(arg)],
        &mut calls,
        &mut memory,
    );
    (result, calls.0, memory.read_bytes(input, 8).unwrap())
}

#[test]
fn eight_byte_table_checks_keep_first_observations_and_fixed_loads() {
    let mut f = Fixture::new();
    f.graph.symbols.push(("decode_table".into(), 0));
    f.graph.data_objects.push(kfrc_body::DataObject {
        symbol: "decode_table".into(),
        bytes: (0..=255).rev().collect(),
        align: 1,
        ..Default::default()
    });
    let table = f.value(Opcode::SymbolAddress, &[], 0, f.pointer);
    let mut result = f.arg;
    let mut checks = Vec::new();
    for offset in 0..8 {
        let index = f.graph.add_constant(offset, f.word);
        let address = f.value(Opcode::IndexAddr, &[f.input, index], 0, f.pointer);
        let byte = f.load(address);
        f.observe(byte);
        let condition = f.condition(byte);
        let branch = f.bounds_check(condition);
        checks.push((branch, condition));
        let index = f.widen(byte);
        let address = f.value(Opcode::IndexAddr, &[table, index], 0, f.pointer);
        let decoded = f.load(address);
        let wide = f.widen(decoded);
        result = f.value(Opcode::Add, &[result, wide], 0, f.word);
    }
    f.ret(result);
    let original = f.graph.clone();
    let stats = f.optimize();
    assert_eq!(stats.conditions_folded, 8);
    assert_eq!(stats.observations_removed, 0);
    for (branch, condition) in checks {
        assert!(stats.dirty_nodes.contains(&branch));
        assert_eq!(f.graph.op(condition), Opcode::Compare);
        assert_eq!(f.graph.resolve(condition), condition);
        assert_eq!(f.graph.constant_value(f.graph.input(branch, 0)), 1);
    }
    assert_eq!(
        crate::transforms::control::simplify::run(&mut f.graph)
            .unwrap()
            .branches_folded,
        8
    );
    verify_graph(&mut f.graph).unwrap();
    assert_eq!(
        f.graph
            .ids()
            .filter(|&n| f.graph.op(n) == Opcode::LoadValue)
            .count(),
        16
    );
    assert_eq!(
        f.graph
            .ids()
            .filter(|&n| f.graph.op(n) == Opcode::RequireDefined)
            .count(),
        8
    );
    assert!(!f.graph.ids().any(|n| f.graph.op(n) == Opcode::Call));
    for input in 0..=255 {
        let bytes = std::array::from_fn(|i| Some((input as u8).wrapping_add(i as u8 * 17)));
        assert_eq!(execute(&f.graph, bytes, 7), execute(&original, bytes, 7));
    }
    for slot in 0..8 {
        let mut bytes = [Some(42); 8];
        bytes[slot] = None;
        let after = execute(&f.graph, bytes, 0);
        assert_eq!(after, execute(&original, bytes, 0));
        assert!(matches!(after.0, Err(InterpretError::PoisonObserved(_))));
        assert!(after.1.is_empty());
    }
}

#[test]
fn missing_or_sibling_observation_does_not_prove_a_table_check() {
    for sibling in [false, true] {
        let mut f = Fixture::new();
        let byte = f.load(f.input);
        if sibling {
            let zero = f.graph.add_constant(0, f.word);
            let select = f.value(Opcode::Compare, &[f.arg, zero], P::Equal as u32, f.boolean);
            let (_, [yes, no]) = f.branch(select);
            let effect = f.effect;
            f.control = yes;
            f.observe(byte);
            let observed = f.effect;
            let left = f.jump();
            f.control = no;
            let right = f.jump();
            let region = f.graph.add_node(
                Opcode::Region,
                NodeId::NONE,
                &[left, right],
                0,
                ValueType::Control,
            );
            f.control = region;
            f.effect = f.graph.add_node(
                Opcode::Phi,
                NodeId::NONE,
                &[region, observed, effect],
                0,
                ValueType::Effect,
            );
        }
        let condition = f.condition(byte);
        let branch = f.bounds_check(condition);
        f.ret(f.arg);
        let original = f.graph.clone();
        assert_eq!(f.optimize().conditions_folded, 0);
        assert_eq!(f.graph.input(branch, 0), condition);
        for arg in [0, 1] {
            for byte in [Some(0), Some(255), None] {
                assert_eq!(
                    execute(&f.graph, [byte; 8], arg),
                    execute(&original, [byte; 8], arg)
                );
            }
        }
    }
}

#[test]
fn a_new_loop_load_cannot_reuse_the_previous_iterations_observation() {
    let mut f = Fixture::new();
    let entry = f.jump();
    let region = f.graph.add_node(
        Opcode::Region,
        NodeId::NONE,
        &[entry, entry],
        0,
        ValueType::Control,
    );
    let zero = f.graph.add_constant(0, f.word);
    let one = f.graph.add_constant(1, f.word);
    let iteration = f.value(Opcode::Phi, &[region, zero, one], 0, f.word);
    let effects = f.graph.add_node(
        Opcode::Phi,
        NodeId::NONE,
        &[region, f.effect, f.effect],
        0,
        ValueType::Effect,
    );
    f.control = region;
    f.effect = effects;
    let byte = f.load(f.input);
    let first = f.value(
        Opcode::Compare,
        &[iteration, zero],
        P::Equal as u32,
        f.boolean,
    );
    let (_, [observe, skip]) = f.branch(first);
    let effect = f.effect;
    let condition = f.condition(byte);
    f.control = observe;
    f.observe(byte);
    let observed_branch = f.bounds_check(condition);
    f.call("invalidate");
    let back = f.jump();
    f.graph.set_input(region, 1, back);
    f.graph.set_input(effects, 2, f.effect);
    f.control = skip;
    f.effect = effect;
    let unobserved_branch = f.bounds_check(condition);
    f.ret(f.arg);
    let original = f.graph.clone();
    assert_eq!(f.optimize().conditions_folded, 1);
    assert_eq!(f.graph.op(f.graph.input(observed_branch, 0)), Opcode::Const);
    assert_eq!(f.graph.input(unobserved_branch, 0), condition);
    let result = execute(&f.graph, [Some(42); 8], 0);
    assert_eq!(result, execute(&original, [Some(42); 8], 0));
    assert!(matches!(result.0, Err(InterpretError::PoisonObserved(_))));
    assert_eq!(result.1, ["invalidate"]);
    assert_eq!(result.2[0], None);
}

#[test]
fn arithmetic_poison_needs_its_own_original_observation() {
    for observe_sum in [false, true] {
        let mut f = Fixture::new();
        let byte = f.load(f.input);
        f.observe(byte);
        let one = f.graph.add_constant(1, f.byte);
        let sum = f.value(Opcode::Add, &[byte, one], ArithmeticFlags::NUW, f.byte);
        if observe_sum {
            f.observe(sum);
        }
        let condition = f.condition(sum);
        f.bounds_check(condition);
        f.ret(f.arg);
        let original = f.graph.clone();
        assert_eq!(f.optimize().conditions_folded, usize::from(observe_sum));
        assert_eq!(
            f.graph
                .ids()
                .filter(|&n| f.graph.op(n) == Opcode::RequireDefined)
                .count(),
            1 + usize::from(observe_sum)
        );
        for byte in [Some(0), Some(254), Some(255), None] {
            assert_eq!(
                execute(&f.graph, [byte; 8], 0),
                execute(&original, [byte; 8], 0)
            );
        }
        assert!(matches!(
            execute(&f.graph, [Some(255); 8], 0).0,
            Err(InterpretError::PoisonObserved(_))
        ));
    }
}

#[test]
fn variant_witness_protects_its_branch_but_not_another_use_of_the_comparison() {
    let mut f = Fixture::new();
    let choice = f.graph.oracle.intern(
        TypeShape::Adt {
            name: "KnownChoice".into(),
        },
        None,
    );
    f.graph.oracle.record_semantic_aggregate(
        choice,
        kfrc_body::SemanticAggregate {
            fields: vec![],
            variants: vec![vec![f.word]],
        },
    );
    f.graph.oracle.record_semantic_enum(choice, vec![0]);
    let value = f.value(Opcode::MakeVariant, &[f.arg], 0, choice);
    let byte = f.value(Opcode::IntCast, &[f.arg], 0, f.byte);
    let condition = f.condition(byte);
    let (protected, [yes, no]) = f.branch(condition);
    f.control = no;
    f.ret(f.arg);
    f.control = yes;
    let witness = f.graph.add_node(
        Opcode::VariantWitness,
        yes,
        &[value, yes],
        0,
        ValueType::Witness,
    );
    let payload = f.graph.intern_variant_field(0, 0);
    let field = f.value(Opcode::VariantField, &[value, witness], payload, f.word);
    let unprotected = f.bounds_check(condition);
    f.ret(field);
    let original = f.graph.clone();
    assert_eq!(f.optimize().conditions_folded, 1);
    assert_eq!(f.graph.input(protected, 0), condition);
    assert_eq!(f.graph.op(f.graph.input(unprotected, 0)), Opcode::Const);
    assert_eq!(f.graph.op(condition), Opcode::Compare);
    for arg in [0, 255, 256, u64::MAX] {
        assert_eq!(
            execute(&f.graph, [Some(0); 8], arg),
            execute(&original, [Some(0); 8], arg)
        );
    }
}

#[test]
fn unsigned_and_disjoint_comparisons_use_entry_definedness_without_an_observation() {
    for predicate in [
        P::UnsignedLess,
        P::UnsignedLessEqual,
        P::UnsignedGreater,
        P::UnsignedGreaterEqual,
        P::Equal,
        P::NotEqual,
        P::SignedLess,
    ] {
        for reverse in [false, true] {
            let mut f = Fixture::new();
            let byte = f.value(Opcode::IntCast, &[f.arg], 0, f.byte);
            let wide = f.widen(byte);
            let limit = f.graph.add_constant(256, f.word);
            let operands = if reverse {
                [limit, wide]
            } else {
                [wide, limit]
            };
            let condition = f.value(Opcode::Compare, &operands, predicate as u32, f.boolean);
            let (_, [yes, no]) = f.branch(condition);
            f.control = yes;
            let one = f.graph.add_constant(1, f.word);
            f.ret(one);
            f.control = no;
            let zero = f.graph.add_constant(0, f.word);
            f.ret(zero);
            let original = f.graph.clone();
            assert_eq!(
                f.optimize().conditions_folded,
                usize::from(predicate != P::SignedLess),
                "{predicate:?} {reverse}"
            );
            for arg in [0, 255, 256, u64::MAX] {
                assert_eq!(
                    execute(&f.graph, [Some(0); 8], arg),
                    execute(&original, [Some(0); 8], arg)
                );
            }
        }
    }
}
