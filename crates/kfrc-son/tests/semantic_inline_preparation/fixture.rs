use kfrc_body::fx_hash::HashMap;
use kfrc_body::{
    FunctionAbi, IntegerWidth, Mutability, PassMode, RecordedTypes, SizeAlign, TyId, TypeShape,
};
use kfrc_son::{
    CallTarget, ComparePredicate, FunctionGraph, GraphLevel, InterpretError, Interpreter, NodeId,
    Opcode, SemanticCallHandler, SemanticMemory, SemanticOutcome, Value, ValueType, verify_graph,
};

pub struct Function {
    pub graph: FunctionGraph,
    pub word: TyId,
    pub control: NodeId,
    pub effect: NodeId,
    pub args: Vec<NodeId>,
}

impl Function {
    pub fn new(name: &str, count: usize) -> Self {
        let mut types = RecordedTypes::new();
        let word = types.intern(
            TypeShape::Int {
                width: IntegerWidth::W64,
                signed: false,
            },
            Some(SizeAlign { size: 8, align: 8 }),
        );
        let mut graph = FunctionGraph::new(
            name,
            FunctionAbi {
                args: vec![PassMode::Direct; count],
                ret: PassMode::Direct,
                c_variadic: false,
            },
            types,
        );
        graph.level = GraphLevel::Specialized;
        let mut outputs = vec![ValueType::Control, ValueType::Effect];
        outputs.extend(std::iter::repeat_n(ValueType::Rust(word), count));
        let start = graph.add_node(
            Opcode::Start,
            NodeId::NONE,
            &[],
            0,
            ValueType::Tuple(outputs),
        );
        graph.start = start;
        let control = projection(&mut graph, start, 0, ValueType::Control);
        let effect = projection(&mut graph, start, 1, ValueType::Effect);
        let args = (0..count)
            .map(|slot| projection(&mut graph, start, slot as u32 + 2, ValueType::Rust(word)))
            .collect();
        Self {
            graph,
            word,
            control,
            effect,
            args,
        }
    }

    pub fn node(&mut self, op: Opcode, inputs: &[NodeId], payload: u32, ty: TyId) -> NodeId {
        self.graph
            .add_node(op, NodeId::NONE, inputs, payload, ValueType::Rust(ty))
    }
    pub fn constant(&mut self, value: u64) -> NodeId {
        self.graph.add_constant(value as u128, self.word)
    }
    pub fn ret(&mut self, value: NodeId) -> NodeId {
        self.graph.add_node(
            Opcode::Return,
            self.control,
            &[self.effect, value],
            0,
            ValueType::Control,
        )
    }
    pub fn call(&mut self, target: &FunctionGraph, args: &[NodeId]) -> NodeId {
        self.call_named(&target.symbol, &target.abi, args)
    }
    pub fn call_named(&mut self, name: &str, abi: &FunctionAbi, args: &[NodeId]) -> NodeId {
        self.call_named_typed(name, abi, args, self.word)
    }
    pub fn call_named_typed(
        &mut self,
        name: &str,
        abi: &FunctionAbi,
        args: &[NodeId],
        result: TyId,
    ) -> NodeId {
        let target = self.graph.add_callee(CallTarget {
            symbol: name.to_owned(),
            abi: abi.clone(),
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
        self.control = projection(&mut self.graph, call, 0, ValueType::Control);
        self.effect = projection(&mut self.graph, call, 1, ValueType::Effect);
        projection(&mut self.graph, call, 2, ValueType::Rust(result))
    }
    pub fn address(&mut self, name: &str, writable: bool) -> NodeId {
        let pointer = self.graph.oracle.intern(
            TypeShape::RawPointer {
                mutability: Mutability::Not,
                pointee: self.word,
            },
            Some(SizeAlign { size: 8, align: 8 }),
        );
        let payload = self.graph.symbols.len() as u32;
        self.graph.symbols.push((name.to_owned(), 0));
        self.graph.data_objects.push(kfrc_body::DataObject {
            symbol: name.to_owned(),
            bytes: vec![0; 8],
            align: 8,
            writable,
            ..Default::default()
        });
        self.node(Opcode::SymbolAddress, &[], payload, pointer)
    }
    pub fn observe(&mut self, value: NodeId) {
        let observe = self.graph.add_node(
            Opcode::RequireDefined,
            self.control,
            &[self.effect, value],
            0,
            ValueType::Tuple(vec![ValueType::Control, ValueType::Effect]),
        );
        self.control = projection(&mut self.graph, observe, 0, ValueType::Control);
        self.effect = projection(&mut self.graph, observe, 1, ValueType::Effect);
    }
    pub fn verify(&mut self) {
        verify_graph(&mut self.graph).unwrap();
    }
}

pub fn projection(graph: &mut FunctionGraph, parent: NodeId, slot: u32, ty: ValueType) -> NodeId {
    graph.add_node(Opcode::Projection, NodeId::NONE, &[parent], slot, ty)
}

pub fn leaf() -> Function {
    let mut f = Function::new("leaf", 2);
    let value = f.node(Opcode::Add, &f.args.clone(), 0, f.word);
    f.ret(value);
    f.verify();
    f
}

/// Literal ASCII selects a small call. The other arm has observable work and
/// identity-bearing data; neither can be discarded from a dynamic instance.
pub fn guarded(name: &str, child: &FunctionGraph) -> Function {
    let mut f = Function::new(name, 2);
    let boolean = f
        .graph
        .oracle
        .intern(TypeShape::Bool, Some(SizeAlign { size: 1, align: 1 }));
    let limit = f.constant(128);
    let condition = f.node(
        Opcode::Compare,
        &[f.args[1], limit],
        ComparePredicate::UnsignedLess as u32,
        boolean,
    );
    let branch = f.graph.add_node(
        Opcode::If,
        f.control,
        &[condition],
        0,
        ValueType::Tuple(vec![ValueType::Control; 2]),
    );
    let entry_effect = f.effect;
    f.control = projection(&mut f.graph, branch, 0, ValueType::Control);
    let value = f.call(child, &f.args[..child.abi.args.len()].to_vec());
    f.ret(value);
    f.control = projection(&mut f.graph, branch, 1, ValueType::Control);
    f.effect = entry_effect;
    let address = f.address(&format!("{name}_cold_identity"), true);
    f.observe(address);
    let abi = FunctionAbi {
        args: vec![PassMode::Direct; 2],
        ret: PassMode::Direct,
        c_variadic: false,
    };
    let mut value = f.call_named("cold", &abi, &[f.args[0], address]);
    for step in 1..40 {
        let constant = f.constant(step);
        value = f.node(Opcode::Add, &[value, constant], 0, f.word);
    }
    f.ret(value);
    f.verify();
    assert!(f.graph.live_count() > 64);
    f
}

/// Five opaque external calls keep this body above the 24-node free limit
/// after scalar cleanup, so the shared eight-helper account limits splicing
/// independently of the nonrefundable node-allocation account.
pub fn costed_helper() -> Function {
    let mut f = Function::new("costed_helper", 1);
    let abi = f.graph.abi.clone();
    let mut value = f.args[0];
    for _ in 0..5 {
        value = f.call_named("touch", &abi, &[value]);
    }
    f.ret(value);
    f.verify();
    assert!(f.graph.live_count() > 24 && f.graph.live_count() <= 64);
    f
}

pub fn open_hot_return(f: &mut Function) -> (NodeId, NodeId) {
    let hot = f
        .graph
        .ids()
        .find(|&node| {
            f.graph.op(node) == Opcode::Return
                && f.graph.op(f.graph.input(node, 1)) == Opcode::Projection
        })
        .unwrap();
    f.control = f.graph.control(hot);
    f.effect = f.graph.input(hot, 0);
    let value = f.graph.input(hot, 1);
    f.graph.remove_node(hot);
    (value, hot)
}

pub fn fanout(name: &str, child: &FunctionGraph, times: usize) -> Function {
    assert!(times > 0);
    let mut f = guarded(name, child);
    let (mut value, _) = open_hot_return(&mut f);
    for _ in 1..times {
        let args = if child.abi.args.len() == 1 {
            vec![value]
        } else {
            vec![value, f.args[1]]
        };
        value = f.call(child, &args);
    }
    f.ret(value);
    f.verify();
    f
}

pub fn chain(depth: usize) -> (HashMap<String, FunctionGraph>, FunctionGraph) {
    let mut graphs = HashMap::default();
    let mut last = leaf().graph;
    graphs.insert(last.symbol.clone(), last.clone());
    for level in (0..depth).rev() {
        let graph = guarded(&format!("wrapper_{level}"), &last).graph;
        graphs.insert(graph.symbol.clone(), graph.clone());
        last = graph;
    }
    (graphs, last)
}

pub fn caller(source: &FunctionGraph, literal: Option<u64>, times: usize) -> Function {
    let mut f = Function::new("caller", if literal.is_some() { 1 } else { 2 });
    let tag = match literal {
        Some(value) => f.constant(value),
        None => f.args[1],
    };
    let mut value = f.args[0];
    for _ in 0..times {
        value = f.call(source, &[value, tag]);
    }
    f.ret(value);
    f.verify();
    f
}

pub fn calls(graph: &FunctionGraph) -> Vec<String> {
    graph
        .ids()
        .filter(|&node| graph.op(node) == Opcode::Call)
        .map(|node| graph.callees[graph.payload(node) as usize].symbol.clone())
        .collect()
}

struct Program<'a> {
    graphs: &'a HashMap<String, FunctionGraph>,
    trace: Vec<String>,
}
impl SemanticCallHandler for Program<'_> {
    fn call(&mut self, _: &str, _: &[Value]) -> Result<Value, InterpretError> {
        unreachable!()
    }
    fn call_with_memory(
        &mut self,
        symbol: &str,
        args: &[Value],
        memory: &mut SemanticMemory,
    ) -> Result<Value, InterpretError> {
        if matches!(symbol, "cold" | "touch" | "foreign_private") {
            self.trace.push(symbol.to_owned());
            return Ok(args[0].clone());
        }
        let graphs = self.graphs;
        let graph = graphs
            .get(symbol)
            .ok_or_else(|| InterpretError::UnknownCallee(symbol.to_owned()))?;
        match Interpreter::new(graph).run_values_in_memory(args, self, memory)? {
            SemanticOutcome::Return(value) => Ok(value),
            SemanticOutcome::Unreachable => Err(InterpretError::UndefinedBehaviour(
                "callee reached unreachable".into(),
            )),
        }
    }
}

pub fn execute(
    graph: &FunctionGraph,
    graphs: &HashMap<String, FunctionGraph>,
    args: &[Value],
) -> (Result<SemanticOutcome, InterpretError>, Vec<String>) {
    let mut memory = SemanticMemory::new();
    let data: Vec<_> = graphs
        .values()
        .chain(std::iter::once(graph))
        .flat_map(|graph| graph.data_objects.clone())
        .collect();
    memory.register_data(&data).unwrap();
    let mut program = Program {
        graphs,
        trace: Vec::new(),
    };
    let result = Interpreter::new(graph).run_values_in_memory(args, &mut program, &mut memory);
    (result, program.trace)
}

pub fn equivalent(
    before: &FunctionGraph,
    after: &FunctionGraph,
    graphs: &HashMap<String, FunctionGraph>,
    args: &[Value],
) {
    let before = execute(before, graphs, args);
    let after = execute(after, graphs, args);
    assert_eq!(before.1, after.1);
    match (&before.0, &after.0) {
        (Err(InterpretError::PoisonObserved(_)), Err(InterpretError::PoisonObserved(_))) => {}
        _ => assert_eq!(before.0, after.0),
    }
}
