use super::*;
use crate::{
    CallTarget, InterpretError, Interpreter, SemanticCallHandler, SemanticMemory, SemanticOutcome,
    Value, verify_graph,
};
use kfrc_body::{
    AggregateLayout, FieldLayout, FunctionAbi, IntegerWidth, PassMode, RecordedTypes, SizeAlign,
};

struct Fixture {
    graph: FunctionGraph,
    word: TyId,
    byte: TyId,
    boolean: TyId,
    unit: TyId,
    pair: TyId,
    nested: TyId,
    root: NodeId,
    other: NodeId,
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
        let byte = types.intern(
            TypeShape::Int {
                width: IntegerWidth::W8,
                signed: false,
            },
            Some(SizeAlign { size: 1, align: 1 }),
        );
        let boolean = types.intern(TypeShape::Bool, Some(SizeAlign { size: 1, align: 1 }));
        let unit = types.intern(TypeShape::Unit, Some(SizeAlign { size: 0, align: 1 }));
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
        let nested = types.intern(
            TypeShape::Tuple(vec![word, pair]),
            Some(SizeAlign { size: 24, align: 8 }),
        );
        types.record_aggregate(
            nested,
            AggregateLayout {
                fields: vec![
                    FieldLayout {
                        ty: word,
                        offset: 0,
                    },
                    FieldLayout {
                        ty: pair,
                        offset: 8,
                    },
                ],
                variants: vec![],
            },
        );
        let pointer = types.intern(
            TypeShape::RawPointer {
                mutability: Mutability::Mut,
                pointee: byte,
            },
            Some(SizeAlign { size: 8, align: 8 }),
        );
        let mut graph = FunctionGraph::new(
            "frozen_ranges",
            FunctionAbi {
                args: vec![PassMode::Direct; 2],
                ret: PassMode::Direct,
                c_variadic: false,
            },
            types,
        );
        graph.level = GraphLevel::Specialized;
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
            ]),
        );
        graph.start = start;
        let mut projection =
            |index, ty| graph.add_node(Opcode::Projection, NodeId::NONE, &[start], index, ty);
        let control = projection(0, ValueType::Control);
        let effect = projection(1, ValueType::Effect);
        let root = projection(2, ValueType::Rust(pointer));
        let other = projection(3, ValueType::Rust(pointer));
        Self {
            graph,
            word,
            byte,
            boolean,
            unit,
            pair,
            nested,
            root,
            other,
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

    fn cast(&mut self, address: NodeId, pointee: TyId) -> NodeId {
        let ty = self.pointer(pointee);
        self.graph.add_node(
            Opcode::PointerCast,
            NodeId::NONE,
            &[address],
            0,
            ValueType::Rust(ty),
        )
    }

    fn reference(&mut self, address: NodeId, pointee: TyId, mutability: Mutability) -> NodeId {
        let ty = self.graph.oracle.intern(
            TypeShape::Reference {
                mutability,
                pointee,
            },
            Some(SizeAlign { size: 8, align: 8 }),
        );
        self.graph.add_node(
            Opcode::PointerCast,
            NodeId::NONE,
            &[address],
            0,
            ValueType::Rust(ty),
        )
    }

    fn field(&mut self, root: NodeId, container: TyId, index: u32, ty: TyId) -> NodeId {
        let base = self.cast(root, container);
        let pointer = self.pointer(ty);
        self.graph.add_node(
            Opcode::FieldAddr,
            NodeId::NONE,
            &[base],
            index,
            ValueType::Rust(pointer),
        )
    }

    fn index(&mut self, root: NodeId, element: TyId, index: NodeId) -> NodeId {
        let base = self.cast(root, element);
        let pointer = self.pointer(element);
        self.graph.add_node(
            Opcode::IndexAddr,
            NodeId::NONE,
            &[base, index],
            0,
            ValueType::Rust(pointer),
        )
    }

    fn fixed(
        &mut self,
        op: Opcode,
        inputs: &[NodeId],
        payload: u32,
        result: Option<TyId>,
    ) -> (NodeId, Option<NodeId>) {
        let mut operands = vec![self.effect];
        operands.extend_from_slice(inputs);
        let mut results = vec![ValueType::Control, ValueType::Effect];
        results.extend(result.map(ValueType::Rust));
        let node = self.graph.add_node(
            op,
            self.control,
            &operands,
            payload,
            ValueType::Tuple(results),
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

    fn store(&mut self, address: NodeId, value: NodeId) {
        self.fixed(Opcode::StoreValue, &[address, value], 0, None);
    }

    fn load(&mut self, address: NodeId, ty: TyId) -> NodeId {
        self.fixed(Opcode::LoadValue, &[address], 0, Some(ty))
            .1
            .unwrap()
    }

    fn call(&mut self, borrowed: NodeId, permission: ArgPermission) -> NodeId {
        let target = self.graph.add_callee(CallTarget {
            symbol: "observe".into(),
            abi: FunctionAbi {
                args: vec![PassMode::Direct],
                ret: PassMode::Ignore,
                c_variadic: false,
            },
        });
        self.graph
            .callee_permissions
            .insert("observe".into(), vec![permission]);
        self.fixed(Opcode::Call, &[borrowed], target, Some(self.unit))
            .0
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

    fn check(&mut self, expected: usize, executions: &[Run]) -> ForwardStats {
        verify_graph(&mut self.graph).unwrap();
        let original = self.graph.clone();
        let stats = run(&mut self.graph);
        assert_eq!(
            stats.loads_forwarded,
            expected,
            "{stats:?}\n{}",
            self.graph.dump()
        );
        verify_graph(&mut self.graph).unwrap();
        for execution in executions {
            assert_eq!(
                execute(&self.graph, execution),
                execute(&original, execution)
            );
        }
        stats
    }
}

fn frozen(bytes: u64) -> ArgPermission {
    ArgPermission::Frozen {
        dereferenceable: Some(bytes),
    }
}

#[derive(Default)]
struct Run {
    alias_roots: bool,
    pointer_slot: bool,
    mutation: Option<(u64, u64)>,
    fail_call: bool,
}

#[derive(Debug, PartialEq, Eq)]
struct Execution {
    result: Result<SemanticOutcome, InterpretError>,
    bytes: Vec<Option<u8>>,
    calls: Vec<Vec<Option<u8>>>,
}

struct Calls<'a> {
    address: u64,
    run: &'a Run,
    observations: Vec<Vec<Option<u8>>>,
}

impl SemanticCallHandler for Calls<'_> {
    fn call(&mut self, _: &str, _: &[Value]) -> Result<Value, InterpretError> {
        unreachable!("semantic calls supply memory")
    }

    fn call_with_memory(
        &mut self,
        _: &str,
        _: &[Value],
        memory: &mut SemanticMemory,
    ) -> Result<Value, InterpretError> {
        self.observations.push(memory.read_bytes(self.address, 64)?);
        if let Some((offset, value)) = self.run.mutation {
            memory.write_bytes(self.address + offset, &value.to_le_bytes().map(Some))?;
        }
        if self.run.fail_call {
            return Err(InterpretError::UnknownCallee("observe failed".into()));
        }
        Ok(Value::Aggregate(vec![]))
    }

    fn call_indirect_with_memory(
        &mut self,
        _: u64,
        arguments: &[Value],
        memory: &mut SemanticMemory,
    ) -> Result<Value, InterpretError> {
        self.call_with_memory("indirect", arguments, memory)
    }
}

fn execute(graph: &FunctionGraph, run: &Run) -> Execution {
    let mut memory = SemanticMemory::new();
    let address = memory.allocate(64, 8, true).unwrap();
    memory.write_bytes(address, &[Some(0x5a); 64]).unwrap();
    if run.pointer_slot {
        memory
            .write_bytes(address, &(address + 32).to_le_bytes().map(Some))
            .unwrap();
    }
    let arguments = [
        Value::Scalar(address),
        Value::Scalar(address + if run.alias_roots { 0 } else { 32 }),
    ];
    let mut calls = Calls {
        address,
        run,
        observations: Vec::new(),
    };
    let result = Interpreter::new(graph).run_values_in_memory(&arguments, &mut calls, &mut memory);
    Execution {
        result,
        bytes: memory.read_bytes(address, 64).unwrap(),
        calls: calls.observations,
    }
}

#[path = "forward_range_tests/boundaries.rs"]
mod boundaries;
#[path = "forward_range_tests/coverage.rs"]
mod coverage;
#[path = "forward_range_tests/errors.rs"]
mod errors;
#[path = "forward_range_tests/function_attrs.rs"]
mod function_attrs;
