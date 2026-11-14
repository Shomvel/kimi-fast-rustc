use super::*;
use crate::{CallTarget, verify_graph};
use kfrc_body::{IntegerWidth, Mutability, PassMode, RecordedTypes, SizeAlign, TyId, TypeShape};

struct Fixture {
    graph: FunctionGraph,
    word: TyId,
    pointer: TyId,
    args: Vec<NodeId>,
    control: NodeId,
    effect: NodeId,
}

impl Fixture {
    fn new(name: &str, parameters: usize) -> Self {
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
                args: vec![PassMode::Direct; parameters],
                ret: PassMode::Direct,
                c_variadic: false,
            },
            types,
        );
        let mut outputs = vec![ValueType::Control, ValueType::Effect];
        outputs.extend(vec![ValueType::Rust(pointer); parameters]);
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
        let args = (0..parameters)
            .map(|index| {
                graph.add_node(
                    Opcode::Projection,
                    NodeId::NONE,
                    &[start],
                    index as u32 + 2,
                    ValueType::Rust(pointer),
                )
            })
            .collect();
        Self {
            graph,
            word,
            pointer,
            args,
            control,
            effect,
        }
    }

    fn constant(&mut self, value: u128) -> NodeId {
        self.graph.add_constant(value, self.word)
    }

    fn fixed(&mut self, op: Opcode, values: &[NodeId], payload: u32, result: bool) -> NodeId {
        let mut inputs = vec![self.effect];
        inputs.extend_from_slice(values);
        let mut outputs = vec![ValueType::Control, ValueType::Effect];
        if result {
            outputs.push(ValueType::Rust(self.word));
        }
        let node = self.graph.add_node(
            op,
            self.control,
            &inputs,
            payload,
            ValueType::Tuple(outputs),
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
        if result {
            self.graph.add_node(
                Opcode::Projection,
                NodeId::NONE,
                &[node],
                2,
                ValueType::Rust(self.word),
            )
        } else {
            node
        }
    }

    fn call(&mut self, symbol: &str, args: &[NodeId]) -> NodeId {
        let target = self.graph.add_callee(CallTarget {
            symbol: symbol.to_owned(),
            abi: FunctionAbi {
                args: vec![PassMode::Direct; args.len()],
                ret: PassMode::Direct,
                c_variadic: false,
            },
        });
        self.fixed(Opcode::Call, args, target, true)
    }

    fn finish(mut self, value: NodeId) -> FunctionGraph {
        self.graph.add_node(
            Opcode::Return,
            self.control,
            &[self.effect, value],
            0,
            ValueType::Control,
        );
        verify_graph(&mut self.graph).unwrap();
        self.graph
    }

    fn finish_constant(mut self) -> FunctionGraph {
        let value = self.constant(7);
        self.finish(value)
    }
}

fn attrs(graphs: Vec<FunctionGraph>) -> BTreeMap<String, FunctionAttrs> {
    infer(&graphs.into_iter().map(|g| (g.symbol.clone(), g)).collect())
}

#[test]
fn scalar_leaf_is_readnone_and_willreturn() {
    let result = attrs(vec![Fixture::new("leaf", 0).finish_constant()]);
    assert_eq!(result["leaf"].memory, MemoryEffects::None);
    assert!(result["leaf"].willreturn);
    assert!(!result["leaf"].noreturn);
}

#[test]
fn pointer_read_and_write_are_separate_from_capture() {
    let mut read = Fixture::new("read", 1);
    let value = read.fixed(Opcode::LoadValue, &[read.args[0]], 0, true);
    let mut write = Fixture::new("write", 1);
    let seven = write.constant(7);
    write.fixed(Opcode::StoreValue, &[write.args[0], seven], 0, false);
    let result = attrs(vec![read.finish(value), write.finish_constant()]);
    assert_eq!(result["read"].memory, MemoryEffects::ReadOnly);
    assert_eq!(
        result["read"].arguments,
        vec![ArgumentAttrs {
            nocapture: true,
            memory: MemoryEffects::ReadOnly
        }]
    );
    assert_eq!(result["write"].memory, MemoryEffects::Unknown);
    assert_eq!(
        result["write"].arguments,
        vec![ArgumentAttrs {
            nocapture: true,
            memory: MemoryEffects::Unknown
        }]
    );
    assert!(result["write"].willreturn);
}

#[test]
fn unescaped_local_writes_are_not_caller_memory() {
    let mut local = Fixture::new("local", 0);
    let address = local.graph.add_node(
        Opcode::LocalSlot,
        NodeId::NONE,
        &[],
        0,
        ValueType::Rust(local.pointer),
    );
    let seven = local.constant(7);
    local.fixed(Opcode::StoreValue, &[address, seven], 0, false);
    let value = local.fixed(Opcode::LoadValue, &[address], 0, true);
    let mut global = Fixture::new("global", 0);
    global.graph.symbols.push(("external_data".into(), 0));
    let address = global.graph.add_node(
        Opcode::SymbolAddress,
        NodeId::NONE,
        &[],
        0,
        ValueType::Rust(global.pointer),
    );
    let seven = global.constant(7);
    global.fixed(Opcode::StoreValue, &[address, seven], 0, false);
    let result = attrs(vec![local.finish(value), global.finish_constant()]);
    assert_eq!(result["local"].memory, MemoryEffects::None);
    assert_eq!(result["global"].memory, MemoryEffects::Unknown);
}

#[test]
fn missing_callee_blocks_memory_capture_and_termination_proofs() {
    let mut f = Fixture::new("caller", 1);
    f.call("missing", &[f.args[0]]);
    let result = attrs(vec![f.finish_constant()]);
    assert_eq!(result["caller"].memory, MemoryEffects::Unknown);
    assert_eq!(result["caller"].arguments, vec![ArgumentAttrs::UNKNOWN]);
    assert!(!result["caller"].willreturn);
}

#[test]
fn matching_callees_propagate_through_derived_pointers() {
    let mut leaf = Fixture::new("leaf", 1);
    let value = leaf.fixed(Opcode::LoadValue, &[leaf.args[0]], 0, true);
    let mut caller = Fixture::new("caller", 1);
    let zero = caller.constant(0);
    let derived = caller.graph.add_node(
        Opcode::IndexAddr,
        NodeId::NONE,
        &[caller.args[0], zero],
        0,
        ValueType::Rust(caller.pointer),
    );
    let value2 = caller.call("leaf", &[derived]);
    let result = attrs(vec![leaf.finish(value), caller.finish(value2)]);
    assert_eq!(result["caller"].memory, MemoryEffects::ReadOnly);
    assert_eq!(result["caller"].arguments, result["leaf"].arguments);
    assert!(result["caller"].willreturn);
}

#[test]
fn abi_mismatch_never_uses_a_known_body() {
    let mut f = Fixture::new("caller", 1);
    let value = f.call("leaf", &[f.args[0]]);
    f.graph.callees[0].abi.ret = PassMode::Ignore;
    let result = attrs(vec![
        Fixture::new("leaf", 1).finish_constant(),
        f.finish(value),
    ]);
    assert_eq!(result["caller"].memory, MemoryEffects::Unknown);
    assert!(!result["caller"].arguments[0].nocapture);
}

#[test]
fn recursive_calls_cannot_prove_their_own_termination() {
    let mut f = Fixture::new("f", 1);
    f.call("g", &[f.args[0]]);
    let mut g = Fixture::new("g", 1);
    g.call("f", &[g.args[0]]);
    let result = attrs(vec![f.finish_constant(), g.finish_constant()]);
    for name in ["f", "g"] {
        assert_eq!(result[name].memory, MemoryEffects::None);
        assert_eq!(result[name].arguments, vec![ArgumentAttrs::UNUSED]);
        assert!(!result[name].willreturn);
        assert!(!result[name].noreturn);
    }
}

#[test]
fn recursive_writes_reach_every_member_and_transitive_caller() {
    let mut f = Fixture::new("f", 1);
    f.call("g", &[f.args[0]]);
    let mut g = Fixture::new("g", 1);
    g.call("f", &[g.args[0]]);
    let seven = g.constant(7);
    g.fixed(Opcode::StoreValue, &[g.args[0], seven], 0, false);
    let mut outer = Fixture::new("outer", 1);
    outer.call("f", &[outer.args[0]]);
    let result = attrs(vec![
        f.finish_constant(),
        g.finish_constant(),
        outer.finish_constant(),
    ]);
    for name in ["f", "g", "outer"] {
        assert_eq!(result[name].memory, MemoryEffects::Unknown);
        assert_eq!(
            result[name].arguments,
            vec![ArgumentAttrs {
                nocapture: true,
                memory: MemoryEffects::Unknown
            }]
        );
    }
}

#[test]
fn captured_pointer_stored_as_a_value_has_no_access_attribute() {
    let mut f = Fixture::new("capture", 1);
    let pointer_pointer = f.graph.oracle.intern(
        TypeShape::RawPointer {
            mutability: Mutability::Mut,
            pointee: f.pointer,
        },
        Some(SizeAlign { size: 8, align: 8 }),
    );
    let local = f.graph.add_node(
        Opcode::LocalSlot,
        NodeId::NONE,
        &[],
        0,
        ValueType::Rust(pointer_pointer),
    );
    f.fixed(Opcode::StoreValue, &[local, f.args[0]], 0, false);
    let result = attrs(vec![f.finish_constant()]);
    assert_eq!(result["capture"].memory, MemoryEffects::None);
    assert_eq!(result["capture"].arguments, vec![ArgumentAttrs::UNKNOWN]);
}

#[test]
fn returned_pointer_is_capture_even_without_a_memory_access() {
    let f = Fixture::new("returned", 1);
    let pointer = f.args[0];
    let result = attrs(vec![f.finish(pointer)]);
    assert_eq!(result["returned"].memory, MemoryEffects::None);
    assert_eq!(result["returned"].arguments, vec![ArgumentAttrs::UNKNOWN]);
}

#[test]
fn local_storage_passed_to_a_call_remains_conservative() {
    let mut f = Fixture::new("caller", 0);
    let address = f.graph.add_node(
        Opcode::LocalSlot,
        NodeId::NONE,
        &[],
        0,
        ValueType::Rust(f.pointer),
    );
    let seven = f.constant(7);
    f.fixed(Opcode::StoreValue, &[address, seven], 0, false);
    f.call("leaf", &[address]);
    let result = attrs(vec![
        f.finish_constant(),
        Fixture::new("leaf", 1).finish_constant(),
    ]);
    assert_eq!(result["caller"].memory, MemoryEffects::Unknown);
    assert!(result["caller"].willreturn);
}

#[test]
fn opaque_and_indirect_calls_remain_barriers() {
    let mut opaque = Fixture::new("opaque", 1);
    let value = opaque.constant(7);
    opaque.fixed(Opcode::Opaque, &[value], 0, true);
    let mut indirect = Fixture::new("indirect", 1);
    indirect.graph.call_abis.push(FunctionAbi {
        args: vec![],
        ret: PassMode::Direct,
        c_variadic: false,
    });
    indirect.fixed(Opcode::CallIndirect, &[indirect.args[0]], 0, true);
    let result = attrs(vec![opaque.finish_constant(), indirect.finish_constant()]);
    for name in ["opaque", "indirect"] {
        assert_eq!(result[name].memory, MemoryEffects::Unknown);
        assert!(!result[name].willreturn);
    }
}

#[test]
fn infinite_cfg_loop_is_noreturn_and_not_willreturn() {
    let mut f = Fixture::new("loop", 0);
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
    let back = f
        .graph
        .add_node(Opcode::Jump, region, &[], 0, ValueType::Control);
    f.graph.set_inputs(region, &[entry, back]);
    let result = attrs(vec![f.graph]);
    assert_eq!(result["loop"].memory, MemoryEffects::None);
    assert!(result["loop"].noreturn);
    assert!(!result["loop"].willreturn);
}

#[test]
fn published_proof_requires_the_original_target_abi() {
    let leaf = Fixture::new("leaf", 0).finish_constant();
    let mut caller = Fixture::new("caller", 0);
    caller.call("leaf", &[]);
    caller.graph.callee_attrs = attrs(vec![leaf]);
    assert!(caller.graph.callee_attrs(0).is_some());
    caller.graph.callees[0].abi.c_variadic = true;
    assert!(caller.graph.callee_attrs(0).is_none());
}

#[test]
fn replaceable_pure_definition_does_not_license_a_no_write_call() {
    let weak = Fixture::new("weak", 1).finish_constant();
    let mut caller = Fixture::new("caller", 1);
    caller.call("weak", &[caller.args[0]]);
    let caller = caller.finish_constant();
    let bodies = BTreeMap::from([("weak".into(), weak), ("caller".into(), caller)]);
    let inferred = infer_exact(&bodies, &BTreeSet::from(["caller".into()]));
    for name in ["caller", "weak"] {
        assert_eq!(inferred[name].memory, MemoryEffects::Unknown);
        assert!(!inferred[name].willreturn);
        assert_eq!(inferred[name].arguments, vec![ArgumentAttrs::UNKNOWN]);
    }
}
