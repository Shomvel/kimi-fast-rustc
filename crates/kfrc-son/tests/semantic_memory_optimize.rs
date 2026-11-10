use kfrc_body::{
    AggregateLayout, FieldLayout, FunctionAbi, IntegerWidth, Mutability, PassMode, RecordedTypes,
    SizeAlign, TyId, TypeShape,
};
use kfrc_son::{
    CallTarget, FunctionGraph, InterpretError, Interpreter, NodeId, Opcode, SemanticCallHandler,
    SemanticMemory, SemanticOutcome, Value, ValueType, optimize_specialized_graph, verify_graph,
};

struct Fixture {
    graph: FunctionGraph,
    word: TyId,
    pair: TyId,
    pointer: TyId,
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
                variants: Vec::new(),
            },
        );
        let pointer = types.intern(
            TypeShape::RawPointer {
                mutability: Mutability::Mut,
                pointee: word,
            },
            Some(SizeAlign { size: 8, align: 8 }),
        );
        let mut graph = FunctionGraph::new(
            "forward",
            FunctionAbi {
                args: Vec::new(),
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
            pair,
            pointer,
            control,
            effect,
        }
    }

    fn pointer_to(&mut self, ty: TyId) -> TyId {
        self.graph.oracle.intern(
            TypeShape::RawPointer {
                mutability: Mutability::Mut,
                pointee: ty,
            },
            Some(SizeAlign { size: 8, align: 8 }),
        )
    }

    fn slot(&mut self, identity: u32, ty: TyId) -> NodeId {
        let pointer = self.pointer_to(ty);
        self.graph.add_node(
            Opcode::LocalSlot,
            NodeId::NONE,
            &[],
            identity,
            ValueType::Rust(pointer),
        )
    }

    fn fixed(
        &mut self,
        opcode: Opcode,
        operands: &[NodeId],
        payload: u32,
        result: Option<TyId>,
    ) -> Option<NodeId> {
        let mut inputs = vec![self.effect];
        inputs.extend_from_slice(operands);
        let mut results = vec![ValueType::Control, ValueType::Effect];
        if let Some(ty) = result {
            results.push(ValueType::Rust(ty));
        }
        let node = self.graph.add_node(
            opcode,
            self.control,
            &inputs,
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
        result.map(|ty| {
            self.graph.add_node(
                Opcode::Projection,
                NodeId::NONE,
                &[node],
                2,
                ValueType::Rust(ty),
            )
        })
    }

    fn store(&mut self, pointer: NodeId, value: NodeId) {
        self.fixed(Opcode::StoreValue, &[pointer, value], 0, None);
    }

    fn load(&mut self, pointer: NodeId, ty: TyId) -> NodeId {
        self.fixed(Opcode::LoadValue, &[pointer], 0, Some(ty))
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
}

fn run(graph: &FunctionGraph) -> Result<SemanticOutcome, InterpretError> {
    Interpreter::new(graph).run_values(&[], &mut |symbol: &str, _: &[Value]| {
        Err(InterpretError::UnknownCallee(symbol.into()))
    })
}

#[test]
fn optimized_aggregate_is_an_immutable_snapshot_of_initialized_leaves() {
    for select_poison in [false, true] {
        let mut f = Fixture::new();
        let slot = f.slot(0, f.pair);
        let seven = f.graph.add_constant(7, f.word);
        let poison = f.graph.add_node(
            Opcode::Undefined,
            NodeId::NONE,
            &[],
            0,
            ValueType::Rust(f.word),
        );
        let pair = f.graph.add_node(
            Opcode::Aggregate,
            NodeId::NONE,
            &[seven, poison],
            0,
            ValueType::Rust(f.pair),
        );
        f.store(slot, pair);
        let snapshot = f.load(slot, f.pair);
        let ninety = f.graph.add_constant(90, f.word);
        let replacement = f.graph.add_node(
            Opcode::Aggregate,
            NodeId::NONE,
            &[ninety, ninety],
            0,
            ValueType::Rust(f.pair),
        );
        f.store(slot, replacement);
        let field = f.graph.add_node(
            Opcode::Field,
            NodeId::NONE,
            &[snapshot],
            u32::from(select_poison),
            ValueType::Rust(f.word),
        );
        f.ret(field);
        verify_graph(&mut f.graph).unwrap();
        let before = run(&f.graph);
        if select_poison {
            assert!(matches!(before, Err(InterpretError::PoisonObserved(_))));
        } else {
            assert_eq!(before, Ok(SemanticOutcome::Return(Value::Scalar(7))));
        }
        optimize_specialized_graph(&mut f.graph).unwrap();
        verify_graph(&mut f.graph).unwrap();
        let after = run(&f.graph);
        if select_poison {
            assert!(matches!(after, Err(InterpretError::PoisonObserved(_))));
        } else {
            assert_eq!(after, before);
        }
        assert!(
            !f.graph
                .ids()
                .any(|node| f.graph.op(node) == Opcode::LoadValue)
        );
        assert_eq!(
            f.graph
                .ids()
                .filter(|node| f.graph.op(*node) == Opcode::StoreValue)
                .count(),
            0
        );
    }
}

struct Mutator {
    calls: usize,
}
impl SemanticCallHandler for Mutator {
    fn call(&mut self, symbol: &str, _: &[Value]) -> Result<Value, InterpretError> {
        Err(InterpretError::UnknownCallee(symbol.into()))
    }

    fn call_with_memory(
        &mut self,
        _: &str,
        arguments: &[Value],
        memory: &mut SemanticMemory,
    ) -> Result<Value, InterpretError> {
        let Value::Scalar(address) = arguments[0] else {
            panic!("pointer argument")
        };
        self.calls += 1;
        memory.write_bytes(
            address,
            &91u64
                .to_le_bytes()
                .into_iter()
                .map(Some)
                .collect::<Vec<_>>(),
        )?;
        Ok(Value::Scalar(0))
    }
}

#[test]
fn unknown_call_can_change_memory_and_blocks_forwarding() {
    let mut f = Fixture::new();
    let slot = f.slot(0, f.word);
    let seven = f.graph.add_constant(7, f.word);
    f.store(slot, seven);
    let target = f.graph.add_callee(CallTarget {
        symbol: "mutate".into(),
        abi: FunctionAbi {
            args: vec![PassMode::Direct],
            ret: PassMode::Direct,
            c_variadic: false,
        },
    });
    f.fixed(Opcode::Call, &[slot], target, Some(f.word));
    let loaded = f.load(slot, f.word);
    f.ret(loaded);
    verify_graph(&mut f.graph).unwrap();
    let mut calls = Mutator { calls: 0 };
    assert_eq!(
        Interpreter::new(&f.graph)
            .run_values(&[], &mut calls)
            .unwrap(),
        SemanticOutcome::Return(Value::Scalar(91))
    );
    assert_eq!(calls.calls, 1);
    optimize_specialized_graph(&mut f.graph).unwrap();
    verify_graph(&mut f.graph).unwrap();
    let mut calls = Mutator { calls: 0 };
    assert_eq!(
        Interpreter::new(&f.graph)
            .run_values(&[], &mut calls)
            .unwrap(),
        SemanticOutcome::Return(Value::Scalar(91))
    );
    assert_eq!(calls.calls, 1);
    assert_eq!(
        f.graph
            .ids()
            .filter(|node| f.graph.op(*node) == Opcode::LoadValue)
            .count(),
        1
    );
}

#[test]
fn separate_storage_identity_never_forwards_the_previous_store() {
    let mut f = Fixture::new();
    let left = f.slot(0, f.word);
    let right = f.slot(1, f.word);
    let seven = f.graph.add_constant(7, f.word);
    let ninety = f.graph.add_constant(90, f.word);
    f.store(left, seven);
    f.store(right, ninety);
    let loaded = f.load(left, f.word);
    f.ret(loaded);
    verify_graph(&mut f.graph).unwrap();
    let before = run(&f.graph);
    let stats = optimize_specialized_graph(&mut f.graph).unwrap();
    assert_eq!(stats.slots_promoted, 2);
    verify_graph(&mut f.graph).unwrap();
    assert_eq!(run(&f.graph), before);
    assert_eq!(before, Ok(SemanticOutcome::Return(Value::Scalar(7))));
    assert!(
        !f.graph
            .ids()
            .any(|node| f.graph.op(node) == Opcode::LoadValue)
    );
}

#[test]
fn lifetime_restart_clears_the_stored_value() {
    let mut f = Fixture::new();
    let slot = f.slot(0, f.word);
    let seven = f.graph.add_constant(7, f.word);
    f.store(slot, seven);
    f.fixed(Opcode::SlotDead, &[slot], 0, None);
    f.fixed(Opcode::SlotLive, &[slot], 0, None);
    let loaded = f.load(slot, f.word);
    f.ret(loaded);
    verify_graph(&mut f.graph).unwrap();
    assert!(matches!(
        run(&f.graph),
        Err(InterpretError::PoisonObserved(_))
    ));
    let stats = optimize_specialized_graph(&mut f.graph).unwrap();
    assert_eq!(stats.slots_promoted, 1);
    verify_graph(&mut f.graph).unwrap();
    assert!(matches!(
        run(&f.graph),
        Err(InterpretError::PoisonObserved(_))
    ));
    assert!(
        !f.graph
            .ids()
            .any(|node| f.graph.op(node) == Opcode::LoadValue)
    );
}

#[test]
fn forwarding_keeps_the_store_address_observation() {
    let mut f = Fixture::new();
    let null = f.graph.add_constant(0, f.pointer);
    let seven = f.graph.add_constant(7, f.word);
    f.store(null, seven);
    let loaded = f.load(null, f.word);
    f.ret(loaded);
    verify_graph(&mut f.graph).unwrap();
    assert!(matches!(
        run(&f.graph),
        Err(InterpretError::UndefinedBehaviour(_))
    ));
    optimize_specialized_graph(&mut f.graph).unwrap();
    verify_graph(&mut f.graph).unwrap();
    assert!(matches!(
        run(&f.graph),
        Err(InterpretError::UndefinedBehaviour(_))
    ));
    assert!(
        f.graph
            .ids()
            .any(|node| f.graph.op(node) == Opcode::StoreValue)
    );
}

#[test]
fn representation_copy_blocks_the_previous_destination_store() {
    let mut f = Fixture::new();
    let source = f.slot(0, f.word);
    let destination = f.slot(1, f.word);
    let seven = f.graph.add_constant(7, f.word);
    let ninety = f.graph.add_constant(90, f.word);
    let one = f.graph.add_constant(1, f.word);
    f.store(source, ninety);
    f.store(destination, seven);
    f.fixed(
        Opcode::CopyNonOverlapping,
        &[source, destination, one],
        0,
        None,
    );
    let loaded = f.load(destination, f.word);
    f.ret(loaded);
    verify_graph(&mut f.graph).unwrap();
    let before = run(&f.graph);
    optimize_specialized_graph(&mut f.graph).unwrap();
    verify_graph(&mut f.graph).unwrap();
    assert_eq!(run(&f.graph), before);
    assert_eq!(before, Ok(SemanticOutcome::Return(Value::Scalar(90))));
    assert!(
        f.graph
            .ids()
            .any(|node| f.graph.op(node) == Opcode::LoadValue)
    );
}

#[test]
fn layout_copy_libcalls_recompute_local_partitions_without_weakening_escape_rules() {
    for opaque in [false, true] {
        let mut f = Fixture::new();
        let ty = if opaque {
            let mu = f.graph.oracle.intern(
                TypeShape::Adt {
                    name: "MaybeUninitWord".into(),
                },
                Some(SizeAlign { size: 8, align: 8 }),
            );
            f.graph.oracle.record_maybe_uninit(mu, f.word);
            mu
        } else {
            f.word
        };
        let source = f.slot(0, ty);
        let destination = f.slot(1, ty);
        for slot in [source, destination] {
            f.fixed(Opcode::SlotLive, &[slot], 0, None);
        }
        let ninety = f.graph.add_constant(90, f.word);
        let initial = if opaque {
            f.fixed(Opcode::TransmuteValue, &[ninety], 0, Some(ty))
                .unwrap()
        } else {
            ninety
        };
        f.store(source, initial);
        if opaque {
            let snapshot = f.load(source, ty);
            f.store(destination, snapshot);
        } else {
            let one = f.graph.add_constant(1, f.word);
            f.fixed(
                Opcode::CopyNonOverlapping,
                &[source, destination, one],
                0,
                None,
            );
        }
        let snapshot = f.load(destination, ty);
        let result = if opaque {
            f.fixed(Opcode::TransmuteValue, &[snapshot], 0, Some(f.word))
                .unwrap()
        } else {
            snapshot
        };
        for slot in [source, destination] {
            f.fixed(Opcode::SlotDead, &[slot], 0, None);
        }
        f.ret(result);
        verify_graph(&mut f.graph).unwrap();
        optimize_specialized_graph(&mut f.graph).unwrap();
        verify_graph(&mut f.graph).unwrap();
        assert_eq!(
            run(&f.graph),
            Ok(SemanticOutcome::Return(Value::Scalar(90)))
        );
        assert!(f.graph.ids().any(|n| matches!(
            f.graph.op(n),
            Opcode::SlotLive | Opcode::SlotDead
        ) && matches!(
            f.graph.memory_partitions[f.graph.payload(n) as usize],
            kfrc_son::MemoryPartition::Local { .. }
        )));
        kfrc_son::lower_layout(&mut f.graph).unwrap();
        verify_graph(&mut f.graph).unwrap();
        let expected_call = if opaque { "memmove" } else { "memcpy" };
        assert!(f.graph.ids().any(|n| f.graph.op(n) == Opcode::Call
            && f.graph.callees[f.graph.payload(n) as usize].symbol == expected_call));
        for node in f.graph.ids().filter(|&n| {
            matches!(
                f.graph.op(n),
                Opcode::Load | Opcode::Store | Opcode::SlotLive | Opcode::SlotDead
            )
        }) {
            assert_eq!(
                f.graph.memory_partitions[f.graph.payload(node) as usize],
                kfrc_son::MemoryPartition::Unknown
            );
        }
        assert_eq!(
            Interpreter::new(&f.graph).run(&[], &mut |_: &str, _: &[u64]| unreachable!()),
            Ok(kfrc_son::Outcome::Return(90))
        );
    }
}

#[test]
fn layout_promotes_private_store_load_and_retains_const_partition() {
    for constant in [false, true] {
        let mut f = Fixture::new();
        let address = if constant {
            f.graph.symbols.push(("constant_word".into(), 0));
            f.graph.data_objects.push(kfrc_body::DataObject {
                symbol: "constant_word".into(),
                bytes: 37u64.to_le_bytes().to_vec(),
                align: 8,
                ..Default::default()
            });
            f.graph.add_node(
                Opcode::SymbolAddress,
                NodeId::NONE,
                &[],
                0,
                ValueType::Rust(f.pointer),
            )
        } else {
            let slot = f.slot(19, f.word);
            let value = f.graph.add_constant(37, f.word);
            f.store(slot, value);
            slot
        };
        let value = f.load(address, f.word);
        f.ret(value);
        verify_graph(&mut f.graph).unwrap();
        kfrc_son::lower_layout(&mut f.graph).unwrap();
        verify_graph(&mut f.graph).unwrap();
        let load = f.graph.ids().find(|&n| f.graph.op(n) == Opcode::Load);
        if constant {
            assert_eq!(
                f.graph.memory_partitions[f.graph.payload(load.unwrap()) as usize],
                kfrc_son::MemoryPartition::Const { symbol: 0 }
            );
        } else {
            assert!(load.is_none());
            assert!(
                !f.graph
                    .ids()
                    .any(|n| matches!(f.graph.op(n), Opcode::Store | Opcode::StackSlot))
            );
        }
        assert_eq!(
            Interpreter::new(&f.graph).run(&[], &mut |_: &str, _: &[u64]| unreachable!()),
            Ok(kfrc_son::Outcome::Return(37))
        );
    }
}

#[test]
fn repeated_typed_load_reuses_snapshot_but_retains_first_observation() {
    let mut f = Fixture::new();
    let slot = f.slot(0, f.word);
    let seven = f.graph.add_constant(7, f.word);
    f.store(slot, seven);
    let target = f.graph.add_callee(CallTarget {
        symbol: "mutate".into(),
        abi: FunctionAbi {
            args: vec![PassMode::Direct],
            ret: PassMode::Direct,
            c_variadic: false,
        },
    });
    f.fixed(Opcode::Call, &[slot], target, Some(f.word));
    let first = f.load(slot, f.word);
    let second = f.load(slot, f.word);
    f.store(slot, seven);
    let sum = f.graph.add_node(
        Opcode::Add,
        NodeId::NONE,
        &[first, second],
        0,
        ValueType::Rust(f.word),
    );
    f.ret(sum);
    verify_graph(&mut f.graph).unwrap();
    let mut calls = Mutator { calls: 0 };
    let expected = Interpreter::new(&f.graph)
        .run_values(&[], &mut calls)
        .unwrap();
    assert_eq!(expected, SemanticOutcome::Return(Value::Scalar(182)));
    let stats = kfrc_son::optimize_graph(&mut f.graph);
    assert_eq!(stats.loads_forwarded, 1);
    verify_graph(&mut f.graph).unwrap();
    assert_eq!(
        f.graph
            .ids()
            .filter(|&n| f.graph.op(n) == Opcode::LoadValue)
            .count(),
        1
    );
    let mut calls = Mutator { calls: 0 };
    assert_eq!(
        Interpreter::new(&f.graph)
            .run_values(&[], &mut calls)
            .unwrap(),
        expected
    );
}

#[test]
fn repeated_load_is_not_reused_across_an_aliasing_call() {
    let mut f = Fixture::new();
    let slot = f.slot(0, f.word);
    let seven = f.graph.add_constant(7, f.word);
    f.store(slot, seven);
    let first = f.load(slot, f.word);
    let target = f.graph.add_callee(CallTarget {
        symbol: "mutate".into(),
        abi: FunctionAbi {
            args: vec![PassMode::Direct],
            ret: PassMode::Direct,
            c_variadic: false,
        },
    });
    f.fixed(Opcode::Call, &[slot], target, Some(f.word));
    let second = f.load(slot, f.word);
    let sum = f.graph.add_node(
        Opcode::Add,
        NodeId::NONE,
        &[first, second],
        0,
        ValueType::Rust(f.word),
    );
    f.ret(sum);
    verify_graph(&mut f.graph).unwrap();
    kfrc_son::optimize_graph(&mut f.graph);
    verify_graph(&mut f.graph).unwrap();
    let mut calls = Mutator { calls: 0 };
    assert_eq!(
        Interpreter::new(&f.graph)
            .run_values(&[], &mut calls)
            .unwrap(),
        SemanticOutcome::Return(Value::Scalar(98))
    );
    assert_eq!(
        f.graph
            .ids()
            .filter(|&n| f.graph.op(n) == Opcode::LoadValue)
            .count(),
        1
    );
}

fn branch_inputs(f: &mut Fixture) -> [NodeId; 2] {
    let boolean = f
        .graph
        .oracle
        .intern(TypeShape::Bool, Some(SizeAlign { size: 1, align: 1 }));
    f.graph.abi.args = vec![PassMode::Direct; 2];
    f.graph.set_value_type(
        f.graph.start,
        ValueType::Tuple(vec![
            ValueType::Control,
            ValueType::Effect,
            ValueType::Rust(boolean),
            ValueType::Rust(boolean),
        ]),
    );
    [2, 3].map(|index| {
        f.graph.add_node(
            Opcode::Projection,
            NodeId::NONE,
            &[f.graph.start],
            index,
            ValueType::Rust(boolean),
        )
    })
}

fn split_if(f: &mut Fixture, condition: NodeId) -> [NodeId; 2] {
    let branch = f.graph.add_node(
        Opcode::If,
        f.control,
        &[condition],
        0,
        ValueType::Tuple(vec![ValueType::Control; 2]),
    );
    [0, 1].map(|index| {
        f.graph.add_node(
            Opcode::Projection,
            NodeId::NONE,
            &[branch],
            index,
            ValueType::Control,
        )
    })
}

fn keep_true_branch(f: &mut Fixture, condition: NodeId, fallback: u128) {
    let [yes, no] = split_if(f, condition);
    f.control = no;
    let fallback = f.graph.add_constant(fallback, f.word);
    f.ret(fallback);
    f.control = yes;
}

fn branch_outcomes(graph: &FunctionGraph) -> Vec<Result<SemanticOutcome, InterpretError>> {
    [(0, 0), (0, 1), (1, 0), (1, 1)]
        .into_iter()
        .map(|(a, b)| {
            let mut interpreter = Interpreter::new(graph);
            interpreter.fuel = 100;
            interpreter
                .run_values(
                    &[Value::Scalar(a), Value::Scalar(b)],
                    &mut Mutator { calls: 0 },
                )
                .map_err(|error| match error {
                    // optimize_graph compacts node IDs. Preserve the semantic failure
                    // class without comparing its old/new diagnostic location; other
                    // errors retain their full details so unexpected failures differ.
                    InterpretError::PoisonObserved(_) => {
                        InterpretError::PoisonObserved(NodeId::NONE)
                    }
                    other => other,
                })
        })
        .collect()
}

#[test]
fn snapshot_forwarding_keeps_each_nested_branch_and_its_effects() {
    for preceding_load in [false, true] {
        let mut f = Fixture::new();
        let conditions = branch_inputs(&mut f);
        let slot = f.slot(0, f.word);
        let seven = f.graph.add_constant(7, f.word);
        f.store(slot, seven);
        if preceding_load {
            let target = f.graph.add_callee(CallTarget {
                symbol: "mutate".into(),
                abi: FunctionAbi {
                    args: vec![PassMode::Direct],
                    ret: PassMode::Direct,
                    c_variadic: false,
                },
            });
            f.fixed(Opcode::Call, &[slot], target, Some(f.word));
            // The call prevents folding this first load into the store.
            f.load(slot, f.word);
        }
        keep_true_branch(&mut f, conditions[0], 98);
        keep_true_branch(&mut f, conditions[1], 99);
        let loaded = f.load(slot, f.word);
        f.ret(loaded);
        verify_graph(&mut f.graph).unwrap();
        let before = branch_outcomes(&f.graph);
        assert_eq!(before[0], Ok(SemanticOutcome::Return(Value::Scalar(98))));
        assert_eq!(before[2], Ok(SemanticOutcome::Return(Value::Scalar(99))));
        assert_eq!(
            before[3],
            Ok(SemanticOutcome::Return(Value::Scalar(if preceding_load {
                91
            } else {
                7
            })))
        );
        let stats = kfrc_son::optimize_graph(&mut f.graph);
        assert_eq!(stats.loads_forwarded, 1);
        verify_graph(&mut f.graph).unwrap();
        assert_eq!(branch_outcomes(&f.graph), before);
    }
}

#[test]
fn branch_snapshot_stops_at_writes_calls_lifetimes_and_requirements() {
    for barrier in [
        Opcode::StoreValue,
        Opcode::Call,
        Opcode::SlotLive,
        Opcode::RequireDefined,
    ] {
        let mut f = Fixture::new();
        let conditions = branch_inputs(&mut f);
        let slot = f.slot(0, f.word);
        let other = f.slot(1, f.word);
        let seven = f.graph.add_constant(7, f.word);
        f.store(slot, seven);
        keep_true_branch(&mut f, conditions[0], 98);
        match barrier {
            Opcode::StoreValue => f.store(other, seven),
            Opcode::Call => {
                let target = f.graph.add_callee(CallTarget {
                    symbol: "mutate".into(),
                    abi: FunctionAbi {
                        args: vec![PassMode::Direct],
                        ret: PassMode::Direct,
                        c_variadic: false,
                    },
                });
                f.fixed(Opcode::Call, &[slot], target, Some(f.word));
            }
            Opcode::SlotLive => {
                f.fixed(Opcode::SlotLive, &[slot], 0, None);
            }
            Opcode::RequireDefined => {
                f.fixed(Opcode::RequireDefined, &[seven], 0, None);
            }
            _ => unreachable!(),
        }
        keep_true_branch(&mut f, conditions[1], 99);
        let loaded = f.load(slot, f.word);
        f.ret(loaded);
        verify_graph(&mut f.graph).unwrap();
        let before = branch_outcomes(&f.graph);
        let stats = kfrc_son::optimize_graph(&mut f.graph);
        assert_eq!(stats.loads_forwarded, 0, "{barrier:?}");
        verify_graph(&mut f.graph).unwrap();
        assert_eq!(branch_outcomes(&f.graph), before, "{barrier:?}");
    }
}

#[test]
fn branch_snapshot_does_not_reinterpret_a_different_address_or_type() {
    for retype in [false, true] {
        let mut f = Fixture::new();
        let conditions = branch_inputs(&mut f);
        let slot = f.slot(0, f.word);
        let seven = f.graph.add_constant(7, f.word);
        let (address, ty) = if retype {
            let signed = f.graph.oracle.intern(
                TypeShape::Int {
                    width: IntegerWidth::W64,
                    signed: true,
                },
                Some(SizeAlign { size: 8, align: 8 }),
            );
            let pointer = f.pointer_to(signed);
            (
                f.graph.add_node(
                    Opcode::PointerCast,
                    NodeId::NONE,
                    &[slot],
                    0,
                    ValueType::Rust(pointer),
                ),
                signed,
            )
        } else {
            let other = f.slot(1, f.word);
            f.store(other, seven);
            (other, f.word)
        };
        f.store(slot, seven);
        keep_true_branch(&mut f, conditions[0], 98);
        let loaded = f.load(address, ty);
        f.ret(loaded);
        verify_graph(&mut f.graph).unwrap();
        let before = branch_outcomes(&f.graph);
        assert_eq!(kfrc_son::optimize_graph(&mut f.graph).loads_forwarded, 0);
        verify_graph(&mut f.graph).unwrap();
        assert_eq!(branch_outcomes(&f.graph), before);
    }
}

#[test]
fn dominating_snapshot_forwards_across_joins_and_loops_without_writes() {
    for backedge in [false, true] {
        let mut f = Fixture::new();
        let conditions = branch_inputs(&mut f);
        let slot = f.slot(0, f.word);
        let seven = f.graph.add_constant(7, f.word);
        f.store(slot, seven);
        if backedge {
            let entry = f
                .graph
                .add_node(Opcode::Jump, f.control, &[], 0, ValueType::Control);
            let region = f.graph.add_node(
                Opcode::Region,
                NodeId::NONE,
                &[entry],
                0,
                ValueType::Control,
            );
            f.control = region;
            let [exit, again] = split_if(&mut f, conditions[0]);
            let jump = f
                .graph
                .add_node(Opcode::Jump, again, &[], 0, ValueType::Control);
            f.graph.set_inputs(region, &[entry, jump]);
            f.control = exit;
        } else {
            let edges = split_if(&mut f, conditions[0]);
            let jumps = edges.map(|control| {
                f.graph
                    .add_node(Opcode::Jump, control, &[], 0, ValueType::Control)
            });
            f.control =
                f.graph
                    .add_node(Opcode::Region, NodeId::NONE, &jumps, 0, ValueType::Control);
        }
        let loaded = f.load(slot, f.word);
        f.ret(loaded);
        verify_graph(&mut f.graph).unwrap();
        let before = branch_outcomes(&f.graph);
        // The store dominates the load and no path between them writes memory.
        assert_eq!(kfrc_son::optimize_graph(&mut f.graph).loads_forwarded, 1);
        verify_graph(&mut f.graph).unwrap();
        assert_eq!(branch_outcomes(&f.graph), before);
    }
}

#[test]
fn a_store_in_one_loop_iteration_never_forwards_to_the_next() {
    let mut f = Fixture::new();
    let conditions = branch_inputs(&mut f);
    let slot = f.slot(0, f.word);
    let seven = f.graph.add_constant(7, f.word);
    let nine = f.graph.add_constant(9, f.word);
    f.store(slot, seven);
    let entry = f
        .graph
        .add_node(Opcode::Jump, f.control, &[], 0, ValueType::Control);
    let region = f.graph.add_node(
        Opcode::Region,
        NodeId::NONE,
        &[entry],
        0,
        ValueType::Control,
    );
    let entry_effect = f.effect;
    let effect = f.graph.add_node(
        Opcode::Phi,
        NodeId::NONE,
        &[region, entry_effect],
        0,
        ValueType::Effect,
    );
    f.control = region;
    f.effect = effect;
    // The header load observes either the entry store or the previous
    // iteration's store, so it has no single available snapshot.
    let header = f.load(slot, f.word);
    f.store(slot, nine);
    let [exit, again] = split_if(&mut f, conditions[0]);
    let jump = f
        .graph
        .add_node(Opcode::Jump, again, &[], 0, ValueType::Control);
    f.graph.set_inputs(region, &[entry, jump]);
    f.graph
        .set_inputs(effect, &[region, entry_effect, f.effect]);
    f.control = exit;
    f.ret(header);
    verify_graph(&mut f.graph).unwrap();
    let before = branch_outcomes(&f.graph);
    assert_eq!(kfrc_son::optimize_graph(&mut f.graph).loads_forwarded, 0);
    verify_graph(&mut f.graph).unwrap();
    assert_eq!(branch_outcomes(&f.graph), before);
}

#[test]
fn snapshot_forwarding_preserves_variant_cases_and_default() {
    let mut f = Fixture::new();
    let enumeration = f.graph.oracle.intern(
        TypeShape::Adt {
            name: "Choice".into(),
        },
        None,
    );
    f.graph.oracle.record_semantic_aggregate(
        enumeration,
        kfrc_body::SemanticAggregate {
            fields: vec![],
            variants: vec![vec![], vec![]],
        },
    );
    f.graph.oracle.record_semantic_enum(enumeration, vec![0, 1]);
    f.graph.abi.args = vec![PassMode::Direct];
    f.graph.set_value_type(
        f.graph.start,
        ValueType::Tuple(vec![
            ValueType::Control,
            ValueType::Effect,
            ValueType::Rust(enumeration),
        ]),
    );
    let input = f.graph.add_node(
        Opcode::Projection,
        NodeId::NONE,
        &[f.graph.start],
        2,
        ValueType::Rust(enumeration),
    );
    let slot = f.slot(0, f.word);
    let seven = f.graph.add_constant(7, f.word);
    f.store(slot, seven);
    // Case 1 is handled by default; both control projections can safely reuse
    // the stored snapshot while remaining separate observable return paths.
    f.graph.variant_switches.push(vec![0]);
    let branch = f.graph.add_node(
        Opcode::VariantSwitch,
        f.control,
        &[input],
        0,
        ValueType::Tuple(vec![ValueType::Control; 2]),
    );
    let effect = f.effect;
    for index in 0..2 {
        f.control = f.graph.add_node(
            Opcode::Projection,
            NodeId::NONE,
            &[branch],
            index,
            ValueType::Control,
        );
        f.effect = effect;
        let value = f.load(slot, f.word);
        let extra = f.graph.add_constant(u128::from(index), f.word);
        let value = f.graph.add_node(
            Opcode::Add,
            NodeId::NONE,
            &[value, extra],
            0,
            ValueType::Rust(f.word),
        );
        f.ret(value);
    }
    let outcomes = |graph: &FunctionGraph| {
        [0, 1].map(|variant| {
            Interpreter::new(graph).run_values(
                &[Value::Variant {
                    variant,
                    fields: vec![],
                }],
                &mut Mutator { calls: 0 },
            )
        })
    };
    verify_graph(&mut f.graph).unwrap();
    let before = outcomes(&f.graph);
    assert_eq!(
        before,
        [
            Ok(SemanticOutcome::Return(Value::Scalar(7))),
            Ok(SemanticOutcome::Return(Value::Scalar(8)))
        ]
    );
    assert_eq!(kfrc_son::optimize_graph(&mut f.graph).loads_forwarded, 2);
    verify_graph(&mut f.graph).unwrap();
    assert_eq!(outcomes(&f.graph), before);
}

#[test]
fn equal_pointer_casts_of_one_address_share_a_snapshot() {
    let mut f = Fixture::new();
    let slot = f.slot(0, f.word);
    let ty = f.graph.value_type(slot).clone();
    let first = f
        .graph
        .add_node(Opcode::PointerCast, NodeId::NONE, &[slot], 0, ty.clone());
    let second = f
        .graph
        .add_node(Opcode::PointerCast, NodeId::NONE, &[slot], 0, ty);
    let seven = f.graph.add_constant(7, f.word);
    f.store(first, seven);
    let loaded = f.load(second, f.word);
    f.ret(loaded);
    verify_graph(&mut f.graph).unwrap();
    let before = run(&f.graph);
    assert_eq!(kfrc_son::optimize_graph(&mut f.graph).loads_forwarded, 1);
    verify_graph(&mut f.graph).unwrap();
    assert_eq!(run(&f.graph), before);
}

#[test]
fn call_snapshot_precedes_users_of_a_forwarded_load_continuation() {
    use kfrc_son::{GraphLevel, Outcome, lower_layout};
    let mut f = Fixture::new();
    f.graph.oracle.record_abi_pair(
        f.pair,
        [
            FieldLayout {
                ty: f.word,
                offset: 0,
            },
            FieldLayout {
                ty: f.word,
                offset: 8,
            },
        ],
    );
    f.graph.level = GraphLevel::Specialized;
    let slot = f.slot(0, f.word);
    let seven = f.graph.add_constant(7, f.word);
    f.store(slot, seven);
    let target = f.graph.add_callee(CallTarget {
        symbol: "pair".into(),
        abi: FunctionAbi {
            args: vec![],
            ret: PassMode::Pair,
            c_variadic: false,
        },
    });
    let pair = f.fixed(Opcode::Call, &[], target, Some(f.pair)).unwrap();
    // The unused read's continuation is forwarded before the call result is
    // decoded. Its consumer must still be redirected past the snapshot loads.
    f.load(slot, f.word);
    let field = f.graph.add_node(
        Opcode::Field,
        NodeId::NONE,
        &[pair],
        0,
        ValueType::Rust(f.word),
    );
    f.store(slot, field);
    let result = f.load(slot, f.word);
    f.ret(result);
    verify_graph(&mut f.graph).unwrap();
    lower_layout(&mut f.graph).unwrap();
    verify_graph(&mut f.graph).unwrap();
    struct Calls;
    impl kfrc_son::interpret::CallHandler for Calls {
        fn call(&mut self, _: &str, _: &[u64]) -> Result<u64, InterpretError> {
            unreachable!()
        }
        fn call_pair(&mut self, symbol: &str, _: &[u64]) -> Result<(u64, u64), InterpretError> {
            assert_eq!(symbol, "pair");
            Ok((42, 73))
        }
    }
    assert_eq!(
        Interpreter::new(&f.graph).run(&[], &mut Calls).unwrap(),
        Outcome::Return(42)
    );
}
