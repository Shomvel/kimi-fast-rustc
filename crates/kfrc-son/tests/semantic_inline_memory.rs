//! Inlining is compared with executing the original callee in shared memory.
use kfrc_body::fx_hash::HashMap;

use kfrc_body::{
    FunctionAbi, IntegerWidth, Mutability, PassMode, RecordedTypes, SizeAlign, TyId, TypeShape,
};
use kfrc_son::{
    CallTarget, FunctionGraph, GraphLevel, InlineBudget, InterpretError, Interpreter, NodeId,
    Opcode, SemanticCallHandler, SemanticMemory, SemanticOutcome, Value, ValueType, inline_calls,
    verify_graph,
};

struct Function {
    graph: FunctionGraph,
    word: TyId,
    pointer: TyId,
    control: NodeId,
    effect: NodeId,
    args: Vec<NodeId>,
}

impl Function {
    fn new(name: &str, pointer_args: &[bool]) -> Self {
        let mut types = RecordedTypes::new();
        let word = types.intern(
            TypeShape::Int {
                width: IntegerWidth::W64,
                signed: false,
            },
            Some(SizeAlign { size: 8, align: 8 }),
        );
        let pointer = types.intern(
            TypeShape::RawPointer {
                mutability: Mutability::Mut,
                pointee: word,
            },
            Some(SizeAlign { size: 8, align: 8 }),
        );
        let mut graph = FunctionGraph::new(
            name,
            FunctionAbi {
                args: vec![PassMode::Direct; pointer_args.len()],
                ret: PassMode::Direct,
                c_variadic: false,
            },
            types,
        );
        graph.level = GraphLevel::Specialized;
        let mut outputs = vec![ValueType::Control, ValueType::Effect];
        outputs.extend(
            pointer_args
                .iter()
                .map(|pointer_arg| ValueType::Rust(if *pointer_arg { pointer } else { word })),
        );
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
        let args = pointer_args
            .iter()
            .enumerate()
            .map(|(i, p)| {
                graph.add_node(
                    Opcode::Projection,
                    NodeId::NONE,
                    &[start],
                    i as u32 + 2,
                    ValueType::Rust(if *p { pointer } else { word }),
                )
            })
            .collect();
        Self {
            graph,
            word,
            pointer,
            control,
            effect,
            args,
        }
    }

    fn node(&mut self, op: Opcode, operands: &[NodeId], payload: u32, ty: TyId) -> NodeId {
        self.graph
            .add_node(op, NodeId::NONE, operands, payload, ValueType::Rust(ty))
    }
    fn constant(&mut self, value: u64) -> NodeId {
        self.graph.add_constant(value as u128, self.word)
    }
    fn slot(&mut self) -> NodeId {
        self.node(Opcode::LocalSlot, &[], 0, self.pointer)
    }
    fn fixed(
        &mut self,
        op: Opcode,
        operands: &[NodeId],
        payload: u32,
        result: Option<TyId>,
    ) -> Option<NodeId> {
        let mut inputs = vec![self.effect];
        inputs.extend_from_slice(operands);
        let mut outputs = vec![ValueType::Control, ValueType::Effect];
        outputs.extend(result.map(ValueType::Rust));
        let fixed = self.graph.add_node(
            op,
            self.control,
            &inputs,
            payload,
            ValueType::Tuple(outputs),
        );
        self.control = self.graph.add_node(
            Opcode::Projection,
            NodeId::NONE,
            &[fixed],
            0,
            ValueType::Control,
        );
        self.effect = self.graph.add_node(
            Opcode::Projection,
            NodeId::NONE,
            &[fixed],
            1,
            ValueType::Effect,
        );
        result.map(|ty| self.node(Opcode::Projection, &[fixed], 2, ty))
    }
    fn load(&mut self, address: NodeId) -> NodeId {
        self.fixed(Opcode::LoadValue, &[address], 0, Some(self.word))
            .unwrap()
    }
    fn store(&mut self, address: NodeId, value: NodeId) {
        self.fixed(Opcode::StoreValue, &[address, value], 0, None);
    }
    fn call(&mut self, callee: &FunctionGraph, args: &[NodeId], result: TyId) -> NodeId {
        let target = self.graph.add_callee(CallTarget {
            symbol: callee.symbol.clone(),
            abi: callee.abi.clone(),
        });
        self.fixed(Opcode::Call, args, target, Some(result))
            .unwrap()
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
    fn branch(&mut self, condition: NodeId) -> [NodeId; 2] {
        let boolean = self
            .graph
            .oracle
            .intern(TypeShape::Bool, Some(SizeAlign { size: 1, align: 1 }));
        let zero = self.constant(0);
        let test = self.node(
            Opcode::Compare,
            &[condition, zero],
            kfrc_son::ComparePredicate::NotEqual as u32,
            boolean,
        );
        let branch = self.graph.add_node(
            Opcode::If,
            self.control,
            &[test],
            0,
            ValueType::Tuple(vec![ValueType::Control, ValueType::Control]),
        );
        [0, 1].map(|arm| {
            self.graph.add_node(
                Opcode::Projection,
                NodeId::NONE,
                &[branch],
                arm,
                ValueType::Control,
            )
        })
    }
}

struct Program<'a> {
    graphs: &'a HashMap<String, FunctionGraph>,
}
impl SemanticCallHandler for Program<'_> {
    fn call(&mut self, _: &str, _: &[Value]) -> Result<Value, InterpretError> {
        unreachable!("calls must share storage")
    }
    fn call_with_memory(
        &mut self,
        symbol: &str,
        args: &[Value],
        memory: &mut SemanticMemory,
    ) -> Result<Value, InterpretError> {
        let graphs = self.graphs;
        let graph = graphs
            .get(symbol)
            .ok_or_else(|| InterpretError::UnknownCallee(symbol.into()))?;
        match Interpreter::new(graph).run_values_in_memory(args, self, memory)? {
            SemanticOutcome::Return(value) => Ok(value),
            SemanticOutcome::Unreachable => Err(InterpretError::UndefinedBehaviour(
                "callee reached unreachable".into(),
            )),
        }
    }
}

fn execute(
    graph: &FunctionGraph,
    graphs: &HashMap<String, FunctionGraph>,
) -> Result<SemanticOutcome, InterpretError> {
    let mut memory = SemanticMemory::new();
    let data = graphs
        .values()
        .chain(std::iter::once(graph))
        .flat_map(|graph| graph.data_objects.clone())
        .collect::<Vec<_>>();
    memory.register_data(&data)?;
    Interpreter::new(graph).run_values_in_memory(&[], &mut Program { graphs }, &mut memory)
}

fn compare(
    mut caller: FunctionGraph,
    callees: Vec<FunctionGraph>,
    expected: Result<Value, &'static str>,
) {
    let graphs = callees
        .into_iter()
        .map(|mut graph| {
            verify_graph(&mut graph).unwrap();
            (graph.symbol.clone(), graph)
        })
        .collect::<HashMap<_, _>>();
    verify_graph(&mut caller).unwrap();
    let before = execute(&caller, &graphs);
    let stats = inline_calls(&mut caller, &graphs, InlineBudget::default()).unwrap();
    assert!(stats.calls_inlined > 0, "{:?}", stats.skipped);
    verify_graph(&mut caller).unwrap();
    let after = execute(&caller, &graphs);
    for outcome in [before, after] {
        match &expected {
            Ok(value) => assert_eq!(outcome.unwrap(), SemanticOutcome::Return(value.clone())),
            Err("poison") => assert!(
                matches!(outcome, Err(InterpretError::PoisonObserved(_))),
                "{outcome:?}"
            ),
            Err("lifetime") => assert!(
                matches!(outcome, Err(InterpretError::UndefinedBehaviour(_))),
                "{outcome:?}"
            ),
            Err(other) => panic!("unexpected error class {other}"),
        }
    }
}

#[test]
fn callee_local_store_load_has_same_value_after_inlining() {
    let mut callee = Function::new("local_value", &[]);
    let slot = callee.slot();
    let value = callee.constant(37);
    callee.store(slot, value);
    let result = callee.load(slot);
    callee.ret(result);
    let mut caller = Function::new("caller", &[]);
    let result = caller.call(&callee.graph, &[], caller.word);
    caller.ret(result);
    compare(caller.graph, vec![callee.graph], Ok(Value::Scalar(37)));
}

#[test]
fn inlined_stack_pointer_is_dead_when_the_call_returns() {
    let mut callee = Function::new("dangling", &[]);
    let slot = callee.slot();
    let value = callee.constant(37);
    callee.store(slot, value);
    callee.ret(slot);
    let mut caller = Function::new("caller", &[]);
    let pointer = caller.call(&callee.graph, &[], caller.pointer);
    let value = caller.load(pointer);
    caller.ret(value);
    compare(caller.graph, vec![callee.graph], Err("lifetime"));
}

#[test]
fn nested_borrowed_write_preserves_the_earlier_owned_snapshot() {
    let mut leaf = Function::new("write", &[true]);
    let value = leaf.constant(9);
    leaf.store(leaf.args[0], value);
    leaf.ret(value);
    let mut middle = Function::new("middle", &[true]);
    let result = middle.call(&leaf.graph, &[middle.args[0]], middle.word);
    middle.ret(result);
    let mut caller = Function::new("caller", &[]);
    let slot = caller.slot();
    let one = caller.constant(1);
    caller.store(slot, one);
    let snapshot = caller.load(slot);
    caller.call(&middle.graph, &[slot], caller.word);
    let current = caller.load(slot);
    let result = caller.node(Opcode::Sub, &[current, snapshot], 0, caller.word);
    caller.ret(result);
    compare(
        caller.graph,
        vec![middle.graph, leaf.graph],
        Ok(Value::Scalar(8)),
    );
}

#[test]
fn inlined_local_is_uninitialized_again_on_the_second_loop_iteration() {
    let mut callee = Function::new("initialize_once", &[false]);
    let slot = callee.slot();
    let [initialize, skip] = callee.branch(callee.args[0]);
    let incoming_effect = callee.effect;
    callee.control = initialize;
    let one = callee.constant(1);
    callee.store(slot, one);
    let written_effect = callee.effect;
    let written = callee.jump();
    callee.control = skip;
    let skipped = callee.jump();
    let region = callee.graph.add_node(
        Opcode::Region,
        NodeId::NONE,
        &[written, skipped],
        0,
        ValueType::Control,
    );
    callee.control = region;
    callee.effect = callee.graph.add_node(
        Opcode::Phi,
        NodeId::NONE,
        &[region, written_effect, incoming_effect],
        0,
        ValueType::Effect,
    );
    let result = callee.load(slot);
    callee.ret(result);

    let mut caller = Function::new("caller", &[]);
    let entry = caller.jump();
    let region = caller.graph.add_node(
        Opcode::Region,
        NodeId::NONE,
        &[entry],
        0,
        ValueType::Control,
    );
    let one = caller.constant(1);
    let zero = caller.constant(0);
    let counter = caller.graph.add_node(
        Opcode::Phi,
        NodeId::NONE,
        &[region, one],
        0,
        ValueType::Rust(caller.word),
    );
    let effects = caller.graph.add_node(
        Opcode::Phi,
        NodeId::NONE,
        &[region, caller.effect],
        0,
        ValueType::Effect,
    );
    caller.control = region;
    caller.effect = effects;
    let result = caller.call(&callee.graph, &[counter], caller.word);
    let back_effect = caller.effect;
    let [repeat, done] = caller.branch(counter);
    caller.control = repeat;
    let back = caller.jump();
    caller.graph.set_inputs(region, &[entry, back]);
    caller.graph.set_inputs(counter, &[region, one, zero]);
    let entry_effect = caller.graph.find_projection(caller.graph.start, 1).unwrap();
    caller
        .graph
        .set_inputs(effects, &[region, entry_effect, back_effect]);
    caller.control = done;
    caller.ret(result);
    compare(caller.graph, vec![callee.graph], Err("poison"));
}
