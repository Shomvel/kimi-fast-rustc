use super::*;
use crate::{CallTarget, InterpretError, Interpreter, SemanticOutcome, Value, verify_graph};
use kfrc_body::{
    AggregateLayout, FieldLayout, FunctionAbi, IntegerWidth, Mutability, PassMode, RecordedTypes,
    SizeAlign, TypeShape,
};

struct Fixture {
    graph: FunctionGraph,
    word: TyId,
    boolean: TyId,
    pair: TyId,
    control: NodeId,
    effect: NodeId,
    condition: NodeId,
    limit: NodeId,
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
        let mut graph = FunctionGraph::new(
            "promotion",
            FunctionAbi {
                args: vec![PassMode::Direct, PassMode::Direct],
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
                ValueType::Rust(boolean),
                ValueType::Rust(word),
            ]),
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
        let condition = graph.add_node(
            Opcode::Projection,
            NodeId::NONE,
            &[start],
            2,
            ValueType::Rust(boolean),
        );
        let limit = graph.add_node(
            Opcode::Projection,
            NodeId::NONE,
            &[start],
            3,
            ValueType::Rust(word),
        );
        Self {
            graph,
            word,
            boolean,
            pair,
            control,
            effect,
            condition,
            limit,
        }
    }

    fn pointer(&mut self, ty: TyId) -> TyId {
        self.graph.oracle.intern(
            TypeShape::RawPointer {
                mutability: Mutability::Mut,
                pointee: ty,
            },
            Some(SizeAlign { size: 8, align: 8 }),
        )
    }

    fn slot(&mut self, local: u32, ty: TyId) -> NodeId {
        let pointer = self.pointer(ty);
        self.graph.add_node(
            Opcode::LocalSlot,
            NodeId::NONE,
            &[],
            local,
            ValueType::Rust(pointer),
        )
    }

    fn field(&mut self, base: NodeId, index: u32, ty: TyId) -> NodeId {
        let pointer = self.pointer(ty);
        self.graph.add_node(
            Opcode::FieldAddr,
            NodeId::NONE,
            &[base],
            index,
            ValueType::Rust(pointer),
        )
    }

    fn fixed(
        &mut self,
        op: Opcode,
        args: &[NodeId],
        payload: u32,
        ty: Option<TyId>,
    ) -> Option<NodeId> {
        let mut inputs = vec![self.effect];
        inputs.extend_from_slice(args);
        let mut results = vec![ValueType::Control, ValueType::Effect];
        if let Some(ty) = ty {
            results.push(ValueType::Rust(ty));
        }
        let node = self.graph.add_node(
            op,
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
        ty.map(|ty| {
            self.graph.add_node(
                Opcode::Projection,
                NodeId::NONE,
                &[node],
                2,
                ValueType::Rust(ty),
            )
        })
    }

    fn store(&mut self, slot: NodeId, value: NodeId) {
        self.fixed(Opcode::StoreValue, &[slot, value], 0, None);
    }
    fn load(&mut self, slot: NodeId, ty: TyId) -> NodeId {
        self.fixed(Opcode::LoadValue, &[slot], 0, Some(ty)).unwrap()
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
    fn branch(&mut self, condition: NodeId) -> (NodeId, NodeId) {
        let branch = self.graph.add_node(
            Opcode::If,
            self.control,
            &[condition],
            0,
            ValueType::Tuple(vec![ValueType::Control, ValueType::Control]),
        );
        let yes = self.graph.add_node(
            Opcode::Projection,
            NodeId::NONE,
            &[branch],
            0,
            ValueType::Control,
        );
        let no = self.graph.add_node(
            Opcode::Projection,
            NodeId::NONE,
            &[branch],
            1,
            ValueType::Control,
        );
        (yes, no)
    }
}

fn execute(
    graph: &FunctionGraph,
    condition: bool,
    limit: u64,
) -> Result<SemanticOutcome, InterpretError> {
    Interpreter::new(graph).run_values(
        &[Value::Scalar(u64::from(condition)), Value::Scalar(limit)],
        &mut |_: &str, _: &[Value]| Ok(Value::Scalar(0)),
    )
}

fn assert_equivalent(f: &mut Fixture, samples: &[(bool, u64)]) -> PromotionStats {
    verify_graph(&mut f.graph).unwrap();
    let before: Vec<_> = samples
        .iter()
        .map(|&(condition, limit)| execute(&f.graph, condition, limit))
        .collect();
    let stats = run(&mut f.graph, None);
    verify_graph(&mut f.graph).unwrap();
    for (&(condition, limit), expected) in samples.iter().zip(before) {
        assert_eq!(execute(&f.graph, condition, limit), expected);
    }
    stats
}

#[test]
fn partial_initialization_stays_confined_to_logical_leaves() {
    for field in [0, 1] {
        let mut f = Fixture::new();
        let slot = f.slot(0, f.pair);
        let address = f.field(slot, 1, f.word);
        let seven = f.graph.add_constant(7, f.word);
        f.store(address, seven);
        let snapshot = f.load(slot, f.pair);
        let value = f.graph.add_node(
            Opcode::Field,
            NodeId::NONE,
            &[snapshot],
            field,
            ValueType::Rust(f.word),
        );
        f.ret(value);
        let stats = assert_equivalent(&mut f, &[(false, 0)]);
        assert_eq!(
            (
                stats.slots_promoted,
                stats.loads_removed,
                stats.stores_removed
            ),
            (1, 1, 1)
        );
        if field == 1 {
            assert_eq!(
                execute(&f.graph, false, 0),
                Ok(SemanticOutcome::Return(Value::Scalar(7)))
            );
        } else {
            assert!(matches!(
                execute(&f.graph, false, 0),
                Err(InterpretError::UndefinedBehaviour(_) | InterpretError::PoisonObserved(_))
            ));
        }
        assert!(
            f.graph
                .ids()
                .any(|node| f.graph.op(node) == Opcode::InsertField)
        );
    }
}

#[test]
fn branch_values_join_in_one_semantic_phi() {
    let mut f = Fixture::new();
    let slot = f.slot(0, f.word);
    let effect = f.effect;
    let (yes, no) = f.branch(f.condition);
    f.control = yes;
    let seven = f.graph.add_constant(7, f.word);
    f.store(slot, seven);
    let left_effect = f.effect;
    let left = f.jump();
    f.control = no;
    f.effect = effect;
    let nine = f.graph.add_constant(9, f.word);
    f.store(slot, nine);
    let right_effect = f.effect;
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
        &[region, left_effect, right_effect],
        0,
        ValueType::Effect,
    );
    let value = f.load(slot, f.word);
    f.ret(value);
    let stats = assert_equivalent(&mut f, &[(false, 0), (true, 0)]);
    assert_eq!(stats.value_phis, 1);
    assert_eq!((stats.loads_removed, stats.stores_removed), (1, 2));
}

#[test]
fn loop_backedge_carries_promoted_storage_value() {
    let mut f = Fixture::new();
    let slot = f.slot(0, f.word);
    let zero = f.graph.add_constant(0, f.word);
    f.store(slot, zero);
    let entry_effect = f.effect;
    let entry = f.jump();
    let region = f
        .graph
        .add_growable_node(Opcode::Region, NodeId::NONE, 2, 0, ValueType::Control);
    let effect_phi = f
        .graph
        .add_growable_node(Opcode::Phi, NodeId::NONE, 3, 0, ValueType::Effect);
    f.control = region;
    f.effect = effect_phi;
    let value = f.load(slot, f.word);
    let iteration_effect = f.effect;
    let condition = f.graph.add_node(
        Opcode::Compare,
        NodeId::NONE,
        &[value, f.limit],
        crate::ComparePredicate::UnsignedLess as u32,
        ValueType::Rust(f.boolean),
    );
    let (body, exit) = f.branch(condition);
    f.control = body;
    let one = f.graph.add_constant(1, f.word);
    let next = f.graph.add_node(
        Opcode::Add,
        NodeId::NONE,
        &[value, one],
        0,
        ValueType::Rust(f.word),
    );
    f.store(slot, next);
    let back_effect = f.effect;
    let back = f.jump();
    f.graph.set_inputs(region, &[entry, back]);
    f.graph
        .set_inputs(effect_phi, &[region, entry_effect, back_effect]);
    f.control = exit;
    f.effect = iteration_effect;
    f.ret(value);
    let stats = assert_equivalent(&mut f, &[(false, 0), (false, 1), (false, 9)]);
    assert_eq!(stats.value_phis, 1);
    assert_eq!(
        (
            stats.slots_promoted,
            stats.loads_removed,
            stats.stores_removed
        ),
        (1, 1, 2)
    );
    assert_eq!(
        execute(&f.graph, false, 9),
        Ok(SemanticOutcome::Return(Value::Scalar(9)))
    );
}

#[test]
fn dead_storage_access_is_not_erased_and_revival_resets_poison() {
    for revive in [false, true] {
        let mut f = Fixture::new();
        let slot = f.slot(0, f.word);
        let seven = f.graph.add_constant(7, f.word);
        f.store(slot, seven);
        f.fixed(Opcode::SlotDead, &[slot], 0, None);
        if revive {
            f.fixed(Opcode::SlotLive, &[slot], 0, None);
        }
        let value = f.load(slot, f.word);
        f.ret(value);
        let stats = assert_equivalent(&mut f, &[(false, 0)]);
        assert_eq!(stats.slots_promoted, usize::from(revive));
        assert!(matches!(
            execute(&f.graph, false, 0),
            Err(InterpretError::UndefinedBehaviour(_) | InterpretError::PoisonObserved(_))
        ));
    }
}

#[test]
fn same_pointee_cast_and_inlined_observation_keep_value_semantics() {
    let mut f = Fixture::new();
    let slot = f.slot(17, f.word);
    let pointer = f.pointer(f.word);
    let cast = f.graph.add_node(
        Opcode::PointerCast,
        NodeId::NONE,
        &[slot],
        0,
        ValueType::Rust(pointer),
    );
    f.fixed(Opcode::RequireDefined, &[cast], 0, None);
    f.store(cast, f.limit);
    let value = f.load(slot, f.word);
    f.ret(value);
    let stats = assert_equivalent(&mut f, &[(false, 37)]);
    assert_eq!(stats.slots_promoted, 1);
    assert!(
        f.graph
            .ids()
            .any(|node| f.graph.op(node) == Opcode::RequireDefined)
    );
}

#[test]
fn dynamic_array_address_keeps_real_storage() {
    let mut f = Fixture::new();
    let array = f.graph.oracle.intern(
        TypeShape::Array {
            element: f.word,
            length: 2,
        },
        Some(SizeAlign { size: 16, align: 8 }),
    );
    let slot = f.slot(0, array);
    let seven = f.graph.add_constant(7, f.word);
    let values = f.graph.add_node(
        Opcode::Aggregate,
        NodeId::NONE,
        &[seven, seven],
        0,
        ValueType::Rust(array),
    );
    f.store(slot, values);
    let pointer = f.pointer(f.word);
    let address = f.graph.add_node(
        Opcode::IndexAddr,
        NodeId::NONE,
        &[slot, f.limit],
        0,
        ValueType::Rust(pointer),
    );
    let value = f.load(address, f.word);
    f.ret(value);
    let stats = assert_equivalent(&mut f, &[(false, 0), (false, 1)]);
    assert_eq!(stats.slots_promoted, 0);
}

#[test]
fn address_packaged_in_call_argument_escapes_whole_function() {
    let mut f = Fixture::new();
    let slot = f.slot(0, f.word);
    let pointer = f.pointer(f.word);
    let wrapper = f.graph.oracle.intern(
        TypeShape::Tuple(vec![pointer]),
        Some(SizeAlign { size: 8, align: 8 }),
    );
    f.graph.oracle.record_aggregate(
        wrapper,
        AggregateLayout {
            fields: vec![FieldLayout {
                ty: pointer,
                offset: 0,
            }],
            variants: vec![],
        },
    );
    let argument = f.graph.add_node(
        Opcode::Aggregate,
        NodeId::NONE,
        &[slot],
        0,
        ValueType::Rust(wrapper),
    );
    let seven = f.graph.add_constant(7, f.word);
    f.store(slot, seven);
    let target = f.graph.add_callee(CallTarget {
        symbol: "escape".into(),
        abi: FunctionAbi {
            args: vec![PassMode::Direct],
            ret: PassMode::Direct,
            c_variadic: false,
        },
    });
    f.fixed(Opcode::Call, &[argument], target, Some(f.word));
    let value = f.load(slot, f.word);
    f.ret(value);
    let stats = assert_equivalent(&mut f, &[(false, 0)]);
    assert_eq!(stats.slots_promoted, 0);
}

#[test]
fn seeded_promotion_only_visits_requested_local_identity() {
    let mut f = Fixture::new();
    let first = f.slot(11, f.word);
    let second = f.slot(29, f.word);
    f.store(first, f.limit);
    f.store(second, f.limit);
    let left = f.load(first, f.word);
    let right = f.load(second, f.word);
    let sum = f.graph.add_node(
        Opcode::Add,
        NodeId::NONE,
        &[left, right],
        0,
        ValueType::Rust(f.word),
    );
    f.ret(sum);
    verify_graph(&mut f.graph).unwrap();
    let stats = run(&mut f.graph, Some(&HashSet::from_iter([29])));
    verify_graph(&mut f.graph).unwrap();
    assert_eq!(
        (
            stats.slots_promoted,
            stats.loads_removed,
            stats.stores_removed
        ),
        (1, 1, 1)
    );
    assert_eq!(
        execute(&f.graph, false, 13),
        Ok(SemanticOutcome::Return(Value::Scalar(26)))
    );
    assert_eq!(
        f.graph
            .ids()
            .filter(|node| f.graph.op(*node) == Opcode::LoadValue)
            .count(),
        1
    );
}

#[test]
fn partition_labels_follow_logical_cast_roots_but_copy_stays_unknown() {
    let mut f = Fixture::new();
    let source = f.slot(5, f.word);
    let destination = f.slot(9, f.word);
    let pointer = f.pointer(f.word);
    let cast = f.graph.add_node(
        Opcode::PointerCast,
        NodeId::NONE,
        &[source],
        0,
        ValueType::Rust(pointer),
    );
    let seven = f.graph.add_constant(7, f.word);
    f.store(cast, seven);
    let count = f.graph.add_constant(1, f.word);
    f.fixed(
        Opcode::CopyNonOverlapping,
        &[source, destination, count],
        0,
        None,
    );
    let value = f.load(destination, f.word);
    f.ret(value);
    verify_graph(&mut f.graph).unwrap();
    let before = execute(&f.graph, false, 0);
    let facts = MemoryFacts::new(&f.graph);
    let stats = super::super::partition::run(&mut f.graph, &facts);
    assert_eq!(stats.local_accesses, 2);
    for node in f.graph.ids() {
        let expected = match f.graph.op(node) {
            Opcode::StoreValue => crate::MemoryPartition::Local { local: 5 },
            Opcode::LoadValue => crate::MemoryPartition::Local { local: 9 },
            Opcode::CopyNonOverlapping => crate::MemoryPartition::Unknown,
            _ => continue,
        };
        assert_eq!(
            f.graph.memory_partitions[f.graph.payload(node) as usize],
            expected
        );
    }
    verify_graph(&mut f.graph).unwrap();
    assert_eq!(execute(&f.graph, false, 0), before);
    assert_eq!(run(&mut f.graph, None).slots_promoted, 0);
}

#[test]
fn const_partition_only_labels_immutable_static_loads() {
    for writable in [false, true] {
        let mut f = Fixture::new();
        let pointer = f.pointer(f.word);
        f.graph.symbols.push(("data".into(), 0));
        f.graph.data_objects.push(kfrc_body::DataObject {
            symbol: "data".into(),
            bytes: 42u64.to_le_bytes().to_vec(),
            align: 8,
            writable,
            ..Default::default()
        });
        let address = f.graph.add_node(
            Opcode::SymbolAddress,
            NodeId::NONE,
            &[],
            0,
            ValueType::Rust(pointer),
        );
        let value = f.load(address, f.word);
        let load = f.graph.input(value, 0);
        f.ret(value);
        let facts = MemoryFacts::new(&f.graph);
        let stats = super::super::partition::run(&mut f.graph, &facts);
        assert_eq!(stats.const_loads, usize::from(!writable));
        let expected = if writable {
            crate::MemoryPartition::Unknown
        } else {
            crate::MemoryPartition::Const { symbol: 0 }
        };
        assert_eq!(
            f.graph.memory_partitions[f.graph.payload(load) as usize],
            expected
        );
        verify_graph(&mut f.graph).unwrap();
        assert_eq!(
            execute(&f.graph, false, 0),
            Ok(SemanticOutcome::Return(Value::Scalar(42)))
        );
    }
}

#[test]
fn pointer_phi_merging_local_roots_cannot_split_alias_order() {
    let mut f = Fixture::new();
    let left = f.slot(0, f.word);
    let right = f.slot(1, f.word);
    let pointer = f.pointer(f.word);
    let seven = f.graph.add_constant(7, f.word);
    let nine = f.graph.add_constant(9, f.word);
    f.store(left, seven);
    f.store(right, nine);
    let (yes, no) = f.branch(f.condition);
    f.control = yes;
    let left_jump = f.jump();
    f.control = no;
    let right_jump = f.jump();
    let region = f.graph.add_node(
        Opcode::Region,
        NodeId::NONE,
        &[left_jump, right_jump],
        0,
        ValueType::Control,
    );
    let selected = f.graph.add_node(
        Opcode::Phi,
        NodeId::NONE,
        &[region, left, right],
        0,
        ValueType::Rust(pointer),
    );
    f.control = region;
    let value = f.load(selected, f.word);
    f.ret(value);
    let facts = MemoryFacts::new(&f.graph);
    assert!(!facts.is_unescaped_local(0));
    assert!(!facts.is_unescaped_local(1));
    let stats = super::super::partition::run(&mut f.graph, &facts);
    assert_eq!(stats.local_accesses, 0);
    assert_eq!(
        assert_equivalent(&mut f, &[(false, 0), (true, 0)]).slots_promoted,
        0
    );
}

#[test]
fn empty_seed_set_skips_provenance_and_control_setup() {
    let mut f = Fixture::new();
    let slot = f.slot(3, f.word);
    f.store(slot, f.limit);
    let value = f.load(slot, f.word);
    f.ret(value);
    let before = f.graph.dump();
    let stats = run(&mut f.graph, Some(&HashSet::default()));
    assert_eq!(
        (stats.node_visits, stats.edge_visits, stats.slots_promoted),
        (0, 0, 0)
    );
    assert_eq!(f.graph.dump(), before);
}

fn enum_type(f: &mut Fixture) -> TyId {
    let ty = f.graph.oracle.intern(
        TypeShape::Adt {
            name: "PayloadChoice".into(),
        },
        Some(SizeAlign { size: 24, align: 8 }),
    );
    f.graph.oracle.record_aggregate(
        ty,
        AggregateLayout {
            fields: vec![],
            variants: vec![
                vec![FieldLayout {
                    ty: f.pair,
                    offset: 8
                }];
                2
            ],
        },
    );
    f.graph.oracle.record_semantic_enum(ty, vec![0, 1]);
    f.graph.oracle.enums.insert(
        ty,
        kfrc_body::EnumLayout {
            tag: Some(FieldLayout {
                ty: f.word,
                offset: 0,
            }),
            encoding: kfrc_body::TagEncoding::Direct {
                discriminants: vec![0, 1],
            },
        },
    );
    ty
}

fn variant(f: &mut Fixture, ty: TyId, tag: u32, value: NodeId) -> NodeId {
    let pair = f.graph.add_node(
        Opcode::Aggregate,
        NodeId::NONE,
        &[value, value],
        0,
        ValueType::Rust(f.pair),
    );
    f.graph.add_node(
        Opcode::MakeVariant,
        NodeId::NONE,
        &[pair],
        tag,
        ValueType::Rust(ty),
    )
}

fn variant_address(f: &mut Fixture, slot: NodeId, variant: u32) -> NodeId {
    let pointer = f.pointer(f.pair);
    let payload = f.graph.intern_variant_field(variant, 0);
    f.graph.add_node(
        Opcode::VariantFieldAddr,
        NodeId::NONE,
        &[slot],
        payload,
        ValueType::Rust(pointer),
    )
}

#[test]
fn whole_enum_stores_and_nested_readonly_payloads_promote_to_semantic_values() {
    for tag in [0, 1] {
        let mut f = Fixture::new();
        let enumeration = enum_type(&mut f);
        let slot = f.slot(0, enumeration);
        let limit = f.limit;
        let value = variant(&mut f, enumeration, tag, limit);
        f.store(slot, value);
        let address = variant_address(&mut f, slot, tag);
        let field = f.field(address, 1, f.word);
        let result = f.load(field, f.word);
        f.ret(result);
        let stats = assert_equivalent(&mut f, &[(false, 0), (true, 37)]);
        assert_eq!(
            (
                stats.slots_promoted,
                stats.stores_removed,
                stats.loads_removed
            ),
            (1, 1, 1)
        );
        let extraction = f
            .graph
            .ids()
            .find(|&n| f.graph.op(n) == Opcode::VariantField)
            .unwrap();
        assert_eq!(
            f.graph.variant_fields[f.graph.payload(extraction) as usize],
            (tag, 0)
        );
        assert_eq!(f.graph.input(extraction, 0), value);
    }
}

#[test]
fn enum_join_keeps_one_whole_value_phi_before_payload_selection() {
    let mut f = Fixture::new();
    let enumeration = enum_type(&mut f);
    let slot = f.slot(0, enumeration);
    let entry_effect = f.effect;
    let (yes, no) = f.branch(f.condition);
    f.control = yes;
    let seven = f.graph.add_constant(7, f.word);
    let value = variant(&mut f, enumeration, 1, seven);
    f.store(slot, value);
    let left_effect = f.effect;
    let left = f.jump();
    f.control = no;
    f.effect = entry_effect;
    let limit = f.limit;
    let value = variant(&mut f, enumeration, 1, limit);
    f.store(slot, value);
    let right_effect = f.effect;
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
        &[region, left_effect, right_effect],
        0,
        ValueType::Effect,
    );
    let payload = variant_address(&mut f, slot, 1);
    let field = f.field(payload, 0, f.word);
    let value = f.load(field, f.word);
    f.ret(value);
    let stats = assert_equivalent(&mut f, &[(false, 0), (false, 91), (true, 91)]);
    assert_eq!((stats.slots_promoted, stats.value_phis), (1, 1));
    let extraction = f
        .graph
        .ids()
        .find(|&n| f.graph.op(n) == Opcode::VariantField)
        .unwrap();
    let source = f.graph.input(extraction, 0);
    assert_eq!(f.graph.op(source), Opcode::Phi);
    assert_eq!(f.graph.rust_type(source), Some(enumeration));
}

#[test]
fn variant_payload_writes_keep_every_access_to_the_slot_in_storage() {
    for nested in [false, true] {
        let mut f = Fixture::new();
        let enumeration = enum_type(&mut f);
        let slot = f.slot(0, enumeration);
        let limit = f.limit;
        let initial = variant(&mut f, enumeration, 0, limit);
        f.store(slot, initial);
        let payload = variant_address(&mut f, slot, 0);
        let seven = f.graph.add_constant(7, f.word);
        if nested {
            let field = f.field(payload, 1, f.word);
            f.store(field, seven);
        } else {
            let pair = f.graph.add_node(
                Opcode::Aggregate,
                NodeId::NONE,
                &[seven, seven],
                0,
                ValueType::Rust(f.pair),
            );
            f.store(payload, pair);
        }
        let snapshot = f.load(slot, enumeration);
        f.ret(snapshot);
        let stats = assert_equivalent(&mut f, &[(false, 5)]);
        assert_eq!(
            (
                stats.slots_promoted,
                stats.stores_removed,
                stats.loads_removed
            ),
            (0, 0, 0)
        );
        assert!(!f.graph.ids().any(|n| f.graph.op(n) == Opcode::InsertField));
    }
}

#[test]
fn enum_tag_writes_and_escaping_payload_addresses_remain_in_storage() {
    for escape in [false, true] {
        let mut f = Fixture::new();
        let enumeration = enum_type(&mut f);
        let slot = f.slot(0, enumeration);
        let limit = f.limit;
        let value = variant(&mut f, enumeration, 0, limit);
        f.store(slot, value);
        if escape {
            let address = variant_address(&mut f, slot, 0);
            let target = f.graph.add_callee(CallTarget {
                symbol: "escape_payload".into(),
                abi: FunctionAbi {
                    args: vec![PassMode::Direct],
                    ret: PassMode::Direct,
                    c_variadic: false,
                },
            });
            f.fixed(Opcode::Call, &[address], target, Some(f.word));
        } else {
            let tag = f.graph.add_constant(1, f.word);
            f.fixed(Opcode::SetVariant, &[slot, tag], 0, None);
        }
        let value = f.load(slot, enumeration);
        f.ret(value);
        let stats = assert_equivalent(&mut f, &[(false, 13)]);
        assert_eq!(stats.slots_promoted, 0);
    }
}

#[test]
fn single_predecessor_regions_preserve_the_original_enum_identity() {
    let mut f = Fixture::new();
    let enumeration = enum_type(&mut f);
    let slot = f.slot(0, enumeration);
    let limit = f.limit;
    let original = variant(&mut f, enumeration, 1, limit);
    f.store(slot, original);
    for _ in 0..3 {
        let jump = f.jump();
        f.control = f
            .graph
            .add_node(Opcode::Region, NodeId::NONE, &[jump], 0, ValueType::Control);
    }
    let payload = variant_address(&mut f, slot, 1);
    let value = f.load(payload, f.pair);
    f.ret(value);
    let stats = assert_equivalent(&mut f, &[(false, 41)]);
    assert_eq!(stats.slots_promoted, 1);
    let field = f
        .graph
        .ids()
        .find(|&n| f.graph.op(n) == Opcode::VariantField)
        .unwrap();
    assert_eq!(f.graph.input(field, 0), original);
}
