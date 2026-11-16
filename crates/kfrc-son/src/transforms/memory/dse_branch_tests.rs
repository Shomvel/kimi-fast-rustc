use super::*;
use crate::{
    ArgPermission, CallTarget, InterpretError, Interpreter, SemanticCallHandler, SemanticMemory,
    SemanticOutcome, Value, verify_graph,
};
use kfrc_body::{
    AggregateLayout, FieldLayout, FunctionAbi, IntegerWidth, Mutability, PassMode, RecordedTypes,
    SizeAlign, TyId, TypeShape,
};

struct Fixture {
    graph: FunctionGraph,
    word: TyId,
    boolean: TyId,
    pair: TyId,
    address: NodeId,
    conditions: Vec<NodeId>,
    control: NodeId,
    effect: NodeId,
}

impl Fixture {
    fn new(condition_count: usize, protected: bool) -> Self {
        let mut types = RecordedTypes::new();
        let word = types.intern(
            TypeShape::Int {
                width: IntegerWidth::W64,
                signed: false,
            },
            Some(SizeAlign { size: 8, align: 8 }),
        );
        let boolean = types.intern(TypeShape::Bool, Some(SizeAlign { size: 1, align: 1 }));
        let pair = types.intern(
            TypeShape::Tuple(vec![word, word]),
            Some(SizeAlign { size: 16, align: 8 }),
        );
        types.record_aggregate(
            pair,
            AggregateLayout {
                fields: vec![
                    FieldLayout {
                        ty: word,
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
        let pointer = types.intern(
            if protected {
                TypeShape::Reference {
                    mutability: Mutability::Mut,
                    pointee: pair,
                }
            } else {
                TypeShape::RawPointer {
                    mutability: Mutability::Mut,
                    pointee: pair,
                }
            },
            Some(SizeAlign { size: 8, align: 8 }),
        );
        let mut graph = FunctionGraph::new(
            "branch_overwrites",
            FunctionAbi {
                args: vec![PassMode::Direct; condition_count + 1],
                ret: PassMode::Direct,
                c_variadic: false,
            },
            types,
        );
        graph.level = GraphLevel::Specialized;
        if protected {
            graph.arg_permissions = vec![ArgPermission::None; condition_count + 1];
            graph.arg_permissions[0] = ArgPermission::Unique {
                dereferenceable: Some(16),
            };
        }
        let mut results = vec![
            ValueType::Control,
            ValueType::Effect,
            ValueType::Rust(pointer),
        ];
        results.extend(vec![ValueType::Rust(boolean); condition_count]);
        let start = graph.add_node(
            Opcode::Start,
            NodeId::NONE,
            &[],
            0,
            ValueType::Tuple(results),
        );
        graph.start = start;
        let mut projection =
            |index, ty| graph.add_node(Opcode::Projection, NodeId::NONE, &[start], index, ty);
        let control = projection(0, ValueType::Control);
        let effect = projection(1, ValueType::Effect);
        let address = projection(2, ValueType::Rust(pointer));
        let conditions = (0..condition_count)
            .map(|index| projection(index as u32 + 3, ValueType::Rust(boolean)))
            .collect();
        Self {
            graph,
            word,
            boolean,
            pair,
            address,
            conditions,
            control,
            effect,
        }
    }

    fn pointer(&mut self, pointee: TyId) -> TyId {
        self.graph.oracle.intern(
            TypeShape::RawPointer {
                mutability: Mutability::Mut,
                pointee,
            },
            Some(SizeAlign { size: 8, align: 8 }),
        )
    }

    fn local(&mut self) -> NodeId {
        let pointer = self.pointer(self.pair);
        self.graph.add_node(
            Opcode::LocalSlot,
            NodeId::NONE,
            &[],
            0,
            ValueType::Rust(pointer),
        )
    }

    fn field(&mut self, root: NodeId, index: u32) -> NodeId {
        let pointer = self.pointer(self.word);
        self.graph.add_node(
            Opcode::FieldAddr,
            NodeId::NONE,
            &[root],
            index,
            ValueType::Rust(pointer),
        )
    }

    fn cast(&mut self, address: NodeId, pointee: TyId) -> NodeId {
        let pointer = self.pointer(pointee);
        self.graph.add_node(
            Opcode::PointerCast,
            NodeId::NONE,
            &[address],
            0,
            ValueType::Rust(pointer),
        )
    }

    fn poison(&mut self, ty: TyId) -> NodeId {
        self.graph
            .add_node(Opcode::Undefined, NodeId::NONE, &[], 0, ValueType::Rust(ty))
    }

    fn fixed(
        &mut self,
        op: Opcode,
        inputs: &[NodeId],
        result: Option<TyId>,
    ) -> (NodeId, Option<NodeId>) {
        let mut operands = vec![self.effect];
        operands.extend_from_slice(inputs);
        let mut results = vec![ValueType::Control, ValueType::Effect];
        results.extend(result.map(ValueType::Rust));
        let node = self
            .graph
            .add_node(op, self.control, &operands, 0, ValueType::Tuple(results));
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
        let value = result.map(|ty| {
            self.graph.add_node(
                Opcode::Projection,
                NodeId::NONE,
                &[node],
                2,
                ValueType::Rust(ty),
            )
        });
        (node, value)
    }

    fn store(&mut self, address: NodeId, value: NodeId) -> NodeId {
        self.fixed(Opcode::StoreValue, &[address, value], None).0
    }

    fn ret(&mut self, value: NodeId) -> NodeId {
        self.graph.add_node(
            Opcode::Return,
            self.control,
            &[self.effect, value],
            0,
            ValueType::Control,
        )
    }

    fn load_return(&mut self, address: NodeId) {
        let value = self
            .fixed(Opcode::LoadValue, &[address], Some(self.word))
            .1
            .unwrap();
        self.ret(value);
    }

    fn branch(&mut self, condition: NodeId) -> [(NodeId, NodeId); 2] {
        let branch = self.graph.add_node(
            Opcode::If,
            self.control,
            &[condition],
            0,
            ValueType::Tuple(vec![ValueType::Control; 2]),
        );
        [0, 1].map(|index| {
            let control = self.graph.add_node(
                Opcode::Projection,
                NodeId::NONE,
                &[branch],
                index,
                ValueType::Control,
            );
            (control, self.effect)
        })
    }

    fn enter(&mut self, (control, effect): (NodeId, NodeId)) {
        self.control = control;
        self.effect = effect;
    }

    fn jump(&mut self) -> NodeId {
        self.graph
            .add_node(Opcode::Jump, self.control, &[], 0, ValueType::Control)
    }

    fn straight_jump(&mut self) {
        let jump = self.jump();
        self.control =
            self.graph
                .add_node(Opcode::Region, NodeId::NONE, &[jump], 0, ValueType::Control);
    }

    fn overwrite_arms(&mut self, states: [(NodeId, NodeId); 2], address: NodeId, value: NodeId) {
        for state in states {
            self.enter(state);
            self.store(address, value);
            self.load_return(address);
        }
    }

    fn check(&mut self, old: NodeId, removed: bool) -> DseStats {
        verify_graph(&mut self.graph).unwrap();
        let original = self.graph.clone();
        let stats = run(&mut self.graph);
        assert_eq!(self.graph.is_dead(old), removed, "{stats:?}");
        verify_graph(&mut self.graph).unwrap();
        for mask in 0..1usize << self.conditions.len() {
            let arguments = (0..self.conditions.len())
                .map(|index| Value::Scalar(((mask >> index) & 1) as u64))
                .collect::<Vec<_>>();
            assert_eq!(
                execute(&self.graph, &arguments, false),
                execute(&original, &arguments, false),
                "condition mask {mask}"
            );
        }
        // Poison parameters are observed at Start, before either version can
        // write memory. This is the specific basis of the argument fast path.
        let poison_arguments = vec![Value::Poison; self.conditions.len()];
        assert_eq!(
            execute(&self.graph, &poison_arguments, false),
            execute(&original, &poison_arguments, false)
        );
        stats
    }
}

#[derive(Debug, PartialEq, Eq)]
struct Execution {
    result: Result<SemanticOutcome, InterpretError>,
    bytes: Vec<Option<u8>>,
    calls: Vec<Vec<Option<u8>>>,
}

struct RecordingCalls {
    address: u64,
    size: u64,
    observations: Vec<Vec<Option<u8>>>,
}

impl SemanticCallHandler for RecordingCalls {
    fn call(&mut self, _: &str, _: &[Value]) -> Result<Value, InterpretError> {
        unreachable!("semantic calls must supply their memory")
    }

    fn call_with_memory(
        &mut self,
        _: &str,
        _: &[Value],
        memory: &mut SemanticMemory,
    ) -> Result<Value, InterpretError> {
        self.observations
            .push(memory.read_bytes(self.address, self.size)?);
        Ok(Value::Scalar(7))
    }
}

fn execute(graph: &FunctionGraph, conditions: &[Value], invalid: bool) -> Execution {
    execute_with_size(graph, conditions, invalid, 16)
}

fn execute_with_size(
    graph: &FunctionGraph,
    conditions: &[Value],
    invalid: bool,
    size: u64,
) -> Execution {
    let mut memory = SemanticMemory::new();
    let address = memory.allocate(size, 8, true).unwrap();
    memory
        .write_bytes(address, &vec![Some(0x5a); size as usize])
        .unwrap();
    let mut arguments = vec![Value::Scalar(if invalid { 0 } else { address })];
    arguments.extend_from_slice(conditions);
    let mut calls = RecordingCalls {
        address,
        size,
        observations: vec![],
    };
    let mut interpreter = Interpreter::new(graph);
    interpreter.fuel = 256;
    let result = interpreter.run_values_in_memory(&arguments, &mut calls, &mut memory);
    Execution {
        result,
        bytes: memory.read_bytes(address, size).unwrap(),
        calls: calls.observations,
    }
}

#[path = "dse_branch_tests/coverage.rs"]
mod coverage;

#[path = "dse_branch_tests/byte_coverage.rs"]
mod byte_coverage;

#[path = "dse_branch_tests/barriers.rs"]
mod barriers;

#[path = "dse_branch_tests/access_errors.rs"]
mod access_errors;

#[path = "dse_branch_tests/value_errors.rs"]
mod value_errors;
