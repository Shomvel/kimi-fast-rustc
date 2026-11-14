use super::*;
use crate::{
    ArgPermission, CallTarget, InterpretError, Interpreter, SemanticOutcome, Value, verify_graph,
};
use kfrc_body::{
    AggregateLayout, FieldLayout, FunctionAbi, IntegerWidth, Mutability, PassMode, RecordedTypes,
    SizeAlign, TyId, TypeShape,
};

struct Fixture {
    graph: FunctionGraph,
    word: TyId,
    pair: TyId,
    control: NodeId,
    effect: NodeId,
    argument: Option<NodeId>,
}

impl Fixture {
    fn new(parameter: bool) -> Self {
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
                variants: vec![],
            },
        );
        let reference = types.intern(
            TypeShape::Reference {
                mutability: Mutability::Mut,
                pointee: pair,
            },
            Some(SizeAlign { size: 8, align: 8 }),
        );
        let mut graph = FunctionGraph::new(
            "memory_ranges",
            FunctionAbi {
                args: vec![PassMode::Direct; usize::from(parameter)],
                ret: PassMode::Direct,
                c_variadic: false,
            },
            types,
        );
        graph.level = GraphLevel::Specialized;
        let mut results = vec![ValueType::Control, ValueType::Effect];
        if parameter {
            results.push(ValueType::Rust(reference));
        }
        let start = graph.add_node(
            Opcode::Start,
            NodeId::NONE,
            &[],
            0,
            ValueType::Tuple(results),
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
        let argument = parameter.then(|| {
            graph.add_node(
                Opcode::Projection,
                NodeId::NONE,
                &[start],
                2,
                ValueType::Rust(reference),
            )
        });
        Self {
            graph,
            word,
            pair,
            control,
            effect,
            argument,
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
    fn local(&mut self, local: u32, ty: TyId) -> NodeId {
        let pointer = self.pointer(ty);
        self.graph.add_node(
            Opcode::LocalSlot,
            NodeId::NONE,
            &[],
            local,
            ValueType::Rust(pointer),
        )
    }
    fn field(&mut self, base: NodeId, field: u32) -> NodeId {
        let pointer = self.pointer(self.word);
        self.graph.add_node(
            Opcode::FieldAddr,
            NodeId::NONE,
            &[base],
            field,
            ValueType::Rust(pointer),
        )
    }
    fn fixed(&mut self, op: Opcode, inputs: &[NodeId], ty: Option<TyId>) -> Option<NodeId> {
        let mut operands = vec![self.effect];
        operands.extend_from_slice(inputs);
        let mut results = vec![ValueType::Control, ValueType::Effect];
        if let Some(ty) = ty {
            results.push(ValueType::Rust(ty));
        }
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
    fn store(&mut self, pointer: NodeId, value: NodeId) {
        self.fixed(Opcode::StoreValue, &[pointer, value], None);
    }
    fn ret(&mut self, value: NodeId) {
        self.graph.add_node(
            Opcode::Return,
            self.control,
            &[self.effect, value],
            0,
            ValueType::Control,
        );
        verify_graph(&mut self.graph).unwrap();
    }
}

fn outcome(graph: &FunctionGraph) -> Result<SemanticOutcome, InterpretError> {
    Interpreter::new(graph).run_values(&[], &mut |symbol: &str, _: &[Value]| {
        Err(InterpretError::UnknownCallee(symbol.into()))
    })
}

#[test]
fn a_complete_overwrite_removes_only_the_old_store_across_disjoint_bytes() {
    for retype in [false, true] {
        let mut f = Fixture::new(false);
        let pair = f.local(0, f.pair);
        let left = f.field(pair, 0);
        let right = f.field(pair, 1);
        let target = if retype {
            let pointer = f.pointer(f.word);
            f.graph.add_node(
                Opcode::PointerCast,
                NodeId::NONE,
                &[left],
                0,
                ValueType::Rust(pointer),
            )
        } else {
            left
        };
        let one = f.graph.add_constant(1, f.word);
        let two = f.graph.add_constant(2, f.word);
        f.store(left, one);
        f.store(right, one);
        f.store(target, two);
        let loaded = f.fixed(Opcode::LoadValue, &[left], Some(f.word)).unwrap();
        f.ret(loaded);
        let before = outcome(&f.graph);
        assert_eq!(run(&mut f.graph).stores_removed, 1);
        verify_graph(&mut f.graph).unwrap();
        assert_eq!(outcome(&f.graph), before);
        assert_eq!(before, Ok(SemanticOutcome::Return(Value::Scalar(2))));
    }
}

#[test]
fn protected_writable_parameter_fields_qualify_but_raw_addresses_do_not() {
    let mut f = Fixture::new(true);
    f.graph.arg_permissions = vec![ArgPermission::Unique {
        dereferenceable: Some(16),
    }];
    let argument = f.argument.unwrap();
    let left = f.field(argument, 0);
    let one = f.graph.add_constant(1, f.word);
    f.store(left, one);
    f.store(left, one);
    f.ret(one);
    assert_eq!(run(&mut f.graph).stores_removed, 1);
    verify_graph(&mut f.graph).unwrap();

    for poison in [false, true] {
        let mut f = Fixture::new(false);
        let pointer = f.pointer(f.word);
        let address = if poison {
            f.graph.add_node(
                Opcode::Undefined,
                NodeId::NONE,
                &[],
                0,
                ValueType::Rust(pointer),
            )
        } else {
            f.graph.add_constant(0, pointer)
        };
        let one = f.graph.add_constant(1, f.word);
        f.store(address, one);
        f.store(address, one);
        f.ret(one);
        let before = outcome(&f.graph);
        assert!(before.is_err());
        assert_eq!(run(&mut f.graph).stores_removed, 0);
        assert_eq!(outcome(&f.graph), before);
    }
}

#[test]
fn reads_requirements_calls_and_lifetimes_stop_the_overwrite_proof() {
    for barrier in [
        Opcode::LoadValue,
        Opcode::RequireDefined,
        Opcode::Call,
        Opcode::SlotLive,
        Opcode::SlotDead,
    ] {
        let mut f = Fixture::new(false);
        let slot = f.local(0, f.word);
        let one = f.graph.add_constant(1, f.word);
        f.store(slot, one);
        match barrier {
            Opcode::LoadValue => {
                f.fixed(barrier, &[slot], Some(f.word));
            }
            Opcode::RequireDefined => {
                f.fixed(barrier, &[one], None);
            }
            Opcode::Call => {
                f.graph.add_callee(CallTarget {
                    symbol: "observe".into(),
                    abi: FunctionAbi {
                        args: vec![],
                        ret: PassMode::Direct,
                        c_variadic: false,
                    },
                });
                f.fixed(barrier, &[], Some(f.word));
            }
            _ => {
                f.fixed(barrier, &[slot], None);
            }
        }
        f.store(slot, one);
        f.ret(one);
        let before = outcome(&f.graph);
        assert_eq!(run(&mut f.graph).stores_removed, 0, "{barrier:?}");
        verify_graph(&mut f.graph).unwrap();
        assert_eq!(outcome(&f.graph), before, "{barrier:?}");
    }
}

#[test]
fn storing_poison_is_not_a_value_observation_and_requirements_are_kept() {
    for observe in [false, true] {
        let mut f = Fixture::new(false);
        let slot = f.local(0, f.word);
        let poison = f.graph.add_node(
            Opcode::Undefined,
            NodeId::NONE,
            &[],
            0,
            ValueType::Rust(f.word),
        );
        let one = f.graph.add_constant(1, f.word);
        f.store(slot, poison);
        if observe {
            f.fixed(Opcode::RequireDefined, &[poison], None);
        }
        f.store(slot, one);
        f.ret(one);
        let before = outcome(&f.graph);
        assert_eq!(run(&mut f.graph).stores_removed, usize::from(!observe));
        verify_graph(&mut f.graph).unwrap();
        assert_eq!(outcome(&f.graph), before);
    }
}

#[test]
fn different_typed_coverage_and_out_of_bounds_local_accesses_stay_in_memory() {
    for outside in [false, true] {
        let mut f = Fixture::new(false);
        let slot = f.local(0, f.pair);
        let pointer = f.pointer(f.word);
        let left = f.graph.add_node(
            Opcode::PointerCast,
            NodeId::NONE,
            &[slot],
            0,
            ValueType::Rust(pointer),
        );
        let index = f.graph.add_constant(if outside { 3 } else { 1 }, f.word);
        let shifted = f.graph.add_node(
            Opcode::IndexAddr,
            NodeId::NONE,
            &[left, index],
            0,
            ValueType::Rust(pointer),
        );
        let one = f.graph.add_constant(1, f.word);
        f.store(shifted, one);
        if outside {
            f.store(shifted, one);
        } else {
            // A whole-pair store is not one typed-word overwrite; padding,
            // inactive payload and partial initialization need their own proof.
            let pair = f.graph.add_node(
                Opcode::Aggregate,
                NodeId::NONE,
                &[one, one],
                0,
                ValueType::Rust(f.pair),
            );
            f.store(slot, pair);
        }
        f.ret(one);
        let before = outcome(&f.graph);
        assert_eq!(run(&mut f.graph).stores_removed, 0);
        assert_eq!(outcome(&f.graph), before);
    }
}

#[test]
fn raw_overwrites_keep_the_first_access_check_and_only_erase_redundant_middle_stores() {
    let mut f = Fixture::new(true);
    let address = f.field(f.argument.unwrap(), 0);
    let one = f.graph.add_constant(1, f.word);
    let two = f.graph.add_constant(2, f.word);
    f.store(address, one);
    f.store(address, two);
    f.store(address, one);
    f.ret(one);
    let stats = run(&mut f.graph);
    assert_eq!(stats.stores_removed, 1);
    assert_eq!(
        f.graph
            .ids()
            .filter(|&node| f.graph.op(node) == Opcode::StoreValue)
            .count(),
        2
    );
    verify_graph(&mut f.graph).unwrap();
}

#[test]
fn a_conditional_overwrite_does_not_delete_a_store_on_the_other_path() {
    let mut f = Fixture::new(false);
    let boolean = f
        .graph
        .oracle
        .intern(TypeShape::Bool, Some(SizeAlign { size: 1, align: 1 }));
    f.graph.abi.args = vec![PassMode::Direct];
    f.graph.set_value_type(
        f.graph.start,
        ValueType::Tuple(vec![
            ValueType::Control,
            ValueType::Effect,
            ValueType::Rust(boolean),
        ]),
    );
    let condition = f.graph.add_node(
        Opcode::Projection,
        NodeId::NONE,
        &[f.graph.start],
        2,
        ValueType::Rust(boolean),
    );
    let slot = f.local(0, f.word);
    let one = f.graph.add_constant(1, f.word);
    let two = f.graph.add_constant(2, f.word);
    f.store(slot, one);
    let branch = f.graph.add_node(
        Opcode::If,
        f.control,
        &[condition],
        0,
        ValueType::Tuple(vec![ValueType::Control; 2]),
    );
    let effect = f.effect;
    for index in [0, 1] {
        f.control = f.graph.add_node(
            Opcode::Projection,
            NodeId::NONE,
            &[branch],
            index,
            ValueType::Control,
        );
        f.effect = effect;
        if index == 0 {
            f.store(slot, two);
        }
        let loaded = f.fixed(Opcode::LoadValue, &[slot], Some(f.word)).unwrap();
        f.graph.add_node(
            Opcode::Return,
            f.control,
            &[f.effect, loaded],
            0,
            ValueType::Control,
        );
    }
    verify_graph(&mut f.graph).unwrap();
    let outcomes = |graph: &FunctionGraph| {
        [0, 1].map(|truth| {
            Interpreter::new(graph).run_values(
                &[Value::Scalar(truth)],
                &mut |_: &str, _: &[Value]| unreachable!(),
            )
        })
    };
    let before = outcomes(&f.graph);
    assert_eq!(run(&mut f.graph).stores_removed, 0);
    verify_graph(&mut f.graph).unwrap();
    assert_eq!(outcomes(&f.graph), before);
}

#[test]
fn range_queries_distinguish_overlap_unknown_indices_unions_and_storage_lifetimes() {
    let mut f = Fixture::new(true);
    let pair = f.argument.unwrap();
    let left = f.field(pair, 0);
    let right = f.field(pair, 1);
    let byte = f.graph.oracle.intern(
        TypeShape::Int {
            width: IntegerWidth::W8,
            signed: false,
        },
        Some(SizeAlign { size: 1, align: 1 }),
    );
    let bytes = f.pointer(byte);
    let bytes = f.graph.add_node(
        Opcode::PointerCast,
        NodeId::NONE,
        &[left],
        0,
        ValueType::Rust(bytes),
    );
    let one = f.graph.add_constant(1, f.word);
    let middle = f.graph.add_node(
        Opcode::IndexAddr,
        NodeId::NONE,
        &[bytes, one],
        0,
        ValueType::Rust(f.graph.rust_type(bytes).unwrap()),
    );
    let word_pointer = f.pointer(f.word);
    let overlap = f.graph.add_node(
        Opcode::PointerCast,
        NodeId::NONE,
        &[middle],
        0,
        ValueType::Rust(word_pointer),
    );
    let unknown = f.graph.add_node(
        Opcode::Undefined,
        NodeId::NONE,
        &[],
        0,
        ValueType::Rust(f.word),
    );
    let dynamic = f.graph.add_node(
        Opcode::IndexAddr,
        NodeId::NONE,
        &[left, unknown],
        0,
        ValueType::Rust(word_pointer),
    );
    let unknown_pointer = f.graph.add_node(
        Opcode::Undefined,
        NodeId::NONE,
        &[],
        0,
        ValueType::Rust(word_pointer),
    );
    let union = f.graph.oracle.intern(
        TypeShape::Adt {
            name: "Overlap".into(),
        },
        Some(SizeAlign { size: 8, align: 8 }),
    );
    f.graph.oracle.record_union(
        union,
        vec![
            FieldLayout {
                ty: f.word,
                offset: 0,
            },
            FieldLayout {
                ty: f.word,
                offset: 0,
            },
        ],
    );
    let union = f.local(0, union);
    let union_field = f.field(union, 0);
    let mut locations = MemoryLocations::new(&f.graph);
    assert!(locations.disjoint(&f.graph, left, right));
    assert!(!locations.disjoint(&f.graph, left, overlap));
    assert!(!locations.disjoint(&f.graph, left, dynamic));
    assert!(!locations.disjoint(&f.graph, left, unknown_pointer));
    assert!(!locations.disjoint(&f.graph, left, union_field));
    assert!(!locations.separate_allocations(&f.graph, left, right));
}

#[test]
fn deeply_nested_address_queries_are_bounded_and_failed_queries_are_memoized() {
    let mut f = Fixture::new(false);
    let slot = f.local(0, f.word);
    let pointer = f.pointer(f.word);
    let mut address = slot;
    for _ in 0..40 {
        address = f.graph.add_node(
            Opcode::PointerCast,
            NodeId::NONE,
            &[address],
            0,
            ValueType::Rust(pointer),
        );
    }
    let mut locations = MemoryLocations::new(&f.graph);
    assert_eq!(locations.location(&f.graph, address), None);
    let visits = locations.node_visits;
    assert_eq!(locations.location(&f.graph, address), None);
    assert_eq!(locations.node_visits, visits);
    assert_eq!(locations.budget_bailouts, 1);
}

#[test]
fn whole_root_lifetime_changes_invalidate_every_field() {
    for field in [0, 1] {
        for events in [
            &[Opcode::SlotDead][..],
            &[Opcode::SlotLive][..],
            &[Opcode::SlotDead, Opcode::SlotLive][..],
        ] {
            let mut f = Fixture::new(false);
            let pair = f.local(0, f.pair);
            let field = f.field(pair, field);
            let one = f.graph.add_constant(1, f.word);
            f.store(field, one);
            for &event in events {
                // The verifier requires an allocation root, never a field.
                f.fixed(event, &[pair], None);
            }
            let loaded = f.fixed(Opcode::LoadValue, &[field], Some(f.word)).unwrap();
            f.ret(loaded);
            let before = outcome(&f.graph);
            assert!(before.is_err());
            assert_eq!(super::super::forward::run(&mut f.graph).loads_forwarded, 0);
            verify_graph(&mut f.graph).unwrap();
            assert_eq!(outcome(&f.graph), before);
        }
    }
}

#[test]
fn a_separate_local_lifetime_does_not_invalidate_the_loaded_allocation() {
    for events in [
        &[Opcode::SlotDead][..],
        &[Opcode::SlotLive][..],
        &[Opcode::SlotDead, Opcode::SlotLive][..],
    ] {
        let mut f = Fixture::new(false);
        let pair = f.local(0, f.pair);
        let other = f.local(1, f.word);
        let field = f.field(pair, 1);
        let one = f.graph.add_constant(1, f.word);
        f.store(field, one);
        for &event in events {
            f.fixed(event, &[other], None);
        }
        let loaded = f.fixed(Opcode::LoadValue, &[field], Some(f.word)).unwrap();
        f.ret(loaded);
        let before = outcome(&f.graph);
        let stats = super::super::forward::run(&mut f.graph);
        assert_eq!(stats.loads_forwarded, 1);
        assert_eq!(stats.disjoint_writes_crossed, events.len());
        verify_graph(&mut f.graph).unwrap();
        assert_eq!(before, Ok(SemanticOutcome::Return(Value::Scalar(1))));
        assert_eq!(outcome(&f.graph), before);
    }
}

#[test]
fn duplicate_local_identity_is_rejected_and_never_proves_independence() {
    let mut f = Fixture::new(false);
    let pair = f.local(0, f.pair);
    let alias = f.local(0, f.pair);
    let field = f.field(pair, 1);
    let alias_field = f.field(alias, 1);
    let one = f.graph.add_constant(1, f.word);
    f.store(field, one);
    let store = f.graph.input(f.effect, 0);
    f.graph.add_node(
        Opcode::Return,
        f.control,
        &[f.effect, one],
        0,
        ValueType::Control,
    );
    // Duplicate source-local identity is not a verified input program. The
    // standalone analysis still must not manufacture allocation independence.
    let error = verify_graph(&mut f.graph).unwrap_err();
    assert!(
        matches!(error, crate::VerifyError::TypeMismatch { detail, .. }
        if detail == "LocalSlot storage identity is duplicated")
    );
    let mut locations = MemoryLocations::new(&f.graph);
    assert!(!locations.separate_allocations(&f.graph, field, alias));
    assert!(!locations.disjoint(&f.graph, field, alias_field));
    assert!(!locations.same_typed_place(&f.graph, field, alias_field));
    assert!(!locations.valid_store(&f.graph, field, store));
}

#[test]
fn whole_root_death_preserves_the_first_invalid_store() {
    let mut f = Fixture::new(false);
    let pair = f.local(0, f.pair);
    let field = f.field(pair, 1);
    let one = f.graph.add_constant(1, f.word);
    let two = f.graph.add_constant(2, f.word);
    f.store(field, one);
    f.fixed(Opcode::SlotDead, &[pair], None);
    f.store(field, one);
    let first_invalid = f.graph.input(f.effect, 0);
    f.store(field, two);
    f.ret(two);
    let mut locations = MemoryLocations::new(&f.graph);
    assert!(!locations.valid_store(&f.graph, field, first_invalid));
    let before = outcome(&f.graph);
    assert!(before.is_err());
    assert_eq!(run(&mut f.graph).stores_removed, 0);
    verify_graph(&mut f.graph).unwrap();
    assert_eq!(outcome(&f.graph), before);
}

#[test]
fn a_root_restart_reestablishes_static_store_validity() {
    let mut f = Fixture::new(false);
    let pair = f.local(0, f.pair);
    let field = f.field(pair, 1);
    let one = f.graph.add_constant(1, f.word);
    let two = f.graph.add_constant(2, f.word);
    f.store(field, one);
    f.fixed(Opcode::SlotDead, &[pair], None);
    f.fixed(Opcode::SlotLive, &[pair], None);
    f.store(field, one);
    let first_restarted = f.graph.input(f.effect, 0);
    f.store(field, two);
    let loaded = f.fixed(Opcode::LoadValue, &[field], Some(f.word)).unwrap();
    f.ret(loaded);
    let mut locations = MemoryLocations::new(&f.graph);
    assert!(locations.valid_store(&f.graph, field, first_restarted));
    let before = outcome(&f.graph);
    assert_eq!(before, Ok(SemanticOutcome::Return(Value::Scalar(2))));
    assert_eq!(run(&mut f.graph).stores_removed, 1);
    verify_graph(&mut f.graph).unwrap();
    assert_eq!(outcome(&f.graph), before);
}

#[test]
fn indexed_ranges_require_canonical_nonnegative_constant_indices() {
    for (width, signed, raw, accepted) in [
        (IntegerWidth::W8, false, 0, true),
        (IntegerWidth::W8, false, 255, true),
        (IntegerWidth::W8, false, 256, false),
        (IntegerWidth::W8, true, 127, true),
        (IntegerWidth::W8, true, 255, false),
        (IntegerWidth::W16, false, 65535, true),
        (IntegerWidth::W16, false, 65536, false),
        (IntegerWidth::W16, true, 65535, false),
        (IntegerWidth::W32, true, u32::MAX as u128, false),
        (IntegerWidth::W64, true, 1u128 << 63, false),
        (IntegerWidth::W64, false, 1u128 << 64, false),
    ] {
        let mut f = Fixture::new(false);
        let pair = f.local(0, f.pair);
        let base = f.field(pair, 0);
        let pointer = f.graph.rust_type(base).unwrap();
        let bytes = u64::from(width.bits(64) / 8);
        let ty = f.graph.oracle.intern(
            TypeShape::Int { width, signed },
            Some(SizeAlign {
                size: bytes,
                align: bytes,
            }),
        );
        let index = f.graph.add_constant(raw, ty);
        let address = f.graph.add_node(
            Opcode::IndexAddr,
            NodeId::NONE,
            &[base, index],
            0,
            ValueType::Rust(pointer),
        );
        let equivalent = f.graph.add_constant(raw, f.word);
        let equivalent = f.graph.add_node(
            Opcode::IndexAddr,
            NodeId::NONE,
            &[base, equivalent],
            0,
            ValueType::Rust(pointer),
        );
        let one = f.graph.add_constant(1, f.word);
        f.ret(one);
        let mut locations = MemoryLocations::new(&f.graph);
        assert_eq!(
            locations.location(&f.graph, address).is_some(),
            accepted,
            "{width:?} signed={signed} raw={raw}"
        );
        assert_eq!(
            locations.same_typed_place(&f.graph, address, equivalent),
            accepted
        );
        if !accepted {
            assert!(!locations.disjoint(&f.graph, address, base));
        }
    }
}

#[test]
fn unsupported_index_width_is_rejected_by_s_and_by_the_standalone_range_query() {
    let mut f = Fixture::new(false);
    let pair = f.local(0, f.pair);
    let base = f.field(pair, 0);
    let pointer = f.graph.rust_type(base).unwrap();
    let wide = f.graph.oracle.intern(
        TypeShape::Int {
            width: IntegerWidth::W128,
            signed: false,
        },
        Some(SizeAlign {
            size: 16,
            align: 16,
        }),
    );
    let index = f.graph.add_constant(0, wide);
    let address = f.graph.add_node(
        Opcode::IndexAddr,
        NodeId::NONE,
        &[base, index],
        0,
        ValueType::Rust(pointer),
    );
    let one = f.graph.add_constant(1, f.word);
    f.graph.add_node(
        Opcode::Return,
        f.control,
        &[f.effect, one],
        0,
        ValueType::Control,
    );
    // This checks analysis conservatism outside the supported S subset. It
    // must never be presented as a verified semantic execution fixture.
    let error = verify_graph(&mut f.graph).unwrap_err();
    assert!(
        matches!(error, crate::VerifyError::TypeMismatch { node, detail }
        if node == index && detail == "type is outside the specialized value graph subset")
    );
    let mut locations = MemoryLocations::new(&f.graph);
    assert_eq!(locations.location(&f.graph, address), None);
    assert!(!locations.disjoint(&f.graph, address, base));
    assert!(!locations.same_typed_place(&f.graph, address, base));
}

#[test]
fn noncanonical_narrow_index_does_not_alias_a_different_normalized_location() {
    for (width, raw) in [(IntegerWidth::W8, 256u64), (IntegerWidth::W16, 65536)] {
        let mut f = Fixture::new(false);
        let array = f.graph.oracle.intern(
            TypeShape::Array {
                element: f.word,
                length: raw + 1,
            },
            Some(SizeAlign {
                size: (raw + 1) * 8,
                align: 8,
            }),
        );
        let array = f.local(0, array);
        let pointer = f.pointer(f.word);
        let base = f.graph.add_node(
            Opcode::PointerCast,
            NodeId::NONE,
            &[array],
            0,
            ValueType::Rust(pointer),
        );
        let bytes = u64::from(width.bits(64) / 8);
        let ty = f.graph.oracle.intern(
            TypeShape::Int {
                width,
                signed: false,
            },
            Some(SizeAlign {
                size: bytes,
                align: bytes,
            }),
        );
        let narrow = f.graph.add_constant(u128::from(raw), ty);
        let normalized = f.graph.add_node(
            Opcode::IndexAddr,
            NodeId::NONE,
            &[base, narrow],
            0,
            ValueType::Rust(pointer),
        );
        let wide = f.graph.add_constant(u128::from(raw), f.word);
        let distant = f.graph.add_node(
            Opcode::IndexAddr,
            NodeId::NONE,
            &[base, wide],
            0,
            ValueType::Rust(pointer),
        );
        let one = f.graph.add_constant(1, f.word);
        let two = f.graph.add_constant(2, f.word);
        // S normalizes the narrow index to zero. Reading it after the wide
        // write must still return the first element, never the distant one.
        f.store(normalized, one);
        f.store(distant, two);
        let loaded = f
            .fixed(Opcode::LoadValue, &[normalized], Some(f.word))
            .unwrap();
        f.ret(loaded);
        let before = outcome(&f.graph);
        assert_eq!(before, Ok(SemanticOutcome::Return(Value::Scalar(1))));
        let mut forwarded = f.graph.clone();
        assert_eq!(
            super::super::forward::run(&mut forwarded).loads_forwarded,
            0
        );
        verify_graph(&mut forwarded).unwrap();
        assert_eq!(outcome(&forwarded), before);
        assert_eq!(run(&mut f.graph).stores_removed, 0);
        verify_graph(&mut f.graph).unwrap();
        assert_eq!(outcome(&f.graph), before);
    }
}

#[test]
fn forwarding_crosses_disjoint_fields_but_not_overlapping_casts() {
    for overlap in [false, true] {
        let mut f = Fixture::new(false);
        let pair = f.local(0, f.pair);
        let left = f.field(pair, 0);
        let right = if overlap {
            let pointer = f.pointer(f.word);
            f.graph.add_node(
                Opcode::PointerCast,
                NodeId::NONE,
                &[left],
                0,
                ValueType::Rust(pointer),
            )
        } else {
            f.field(pair, 1)
        };
        let one = f.graph.add_constant(1, f.word);
        let two = f.graph.add_constant(2, f.word);
        f.store(left, one);
        f.store(right, two);
        let loaded = f.fixed(Opcode::LoadValue, &[left], Some(f.word)).unwrap();
        f.ret(loaded);
        let before = outcome(&f.graph);
        let stats = super::super::forward::run(&mut f.graph);
        assert_eq!(stats.loads_forwarded, 1);
        assert_eq!(stats.disjoint_writes_crossed, usize::from(!overlap));
        verify_graph(&mut f.graph).unwrap();
        assert_eq!(outcome(&f.graph), before);
    }
}
