use super::*;
use crate::analysis::function_attrs::{FunctionAttrs, MemoryEffects};

fn set_attrs(f: &mut Fixture, call: NodeId, memory: MemoryEffects) {
    let target = &f.graph.callees[f.graph.payload(call) as usize];
    f.graph.callee_attrs.insert(
        target.symbol.clone(),
        FunctionAttrs {
            abi: target.abi.clone(),
            memory,
            arguments: vec![],
            noreturn: false,
            willreturn: false,
        },
    );
}

#[test]
fn readonly_call_reuses_value_but_preserves_call_observation_and_failure() {
    let mut f = Fixture::new();
    let root = f.cast(f.root, f.word);
    let seven = f.graph.add_constant(7, f.word);
    f.store(root, seven);
    let call = f.call(root, ArgPermission::None);
    set_attrs(&mut f, call, MemoryEffects::ReadOnly);
    let loaded = f.load(root, f.word);
    f.ret(loaded);
    let effect = f.effect;
    let stats = f.check(
        1,
        &[
            Run::default(),
            Run {
                fail_call: true,
                ..Run::default()
            },
        ],
    );
    assert_eq!(stats.calls_crossed_by_forwarding, 1);
    assert_eq!(stats.call_permissions_used, 0);
    assert!(f.graph.ids().any(|node| node == call));
    // The removed load continues from the call; it never restores the
    // earlier store's effect and cannot bypass a failing call.
    assert_eq!(f.graph.op(f.graph.resolve(effect)), Opcode::Projection);
    assert_eq!(
        f.graph.resolve(f.graph.input(f.graph.resolve(effect), 0)),
        call
    );
}

#[test]
fn unknown_write_proof_blocks_reuse_and_preserves_mutation() {
    let mut f = Fixture::new();
    let root = f.cast(f.root, f.word);
    let seven = f.graph.add_constant(7, f.word);
    f.store(root, seven);
    let call = f.call(root, ArgPermission::None);
    set_attrs(&mut f, call, MemoryEffects::Unknown);
    let loaded = f.load(root, f.word);
    f.ret(loaded);
    f.check(
        0,
        &[Run {
            mutation: Some((0, 91)),
            ..Run::default()
        }],
    );
}

#[test]
fn stale_abi_no_write_proof_is_rejected() {
    let mut f = Fixture::new();
    let root = f.cast(f.root, f.word);
    let seven = f.graph.add_constant(7, f.word);
    f.store(root, seven);
    let call = f.call(root, ArgPermission::None);
    set_attrs(&mut f, call, MemoryEffects::None);
    f.graph
        .callee_attrs
        .get_mut("observe")
        .unwrap()
        .abi
        .c_variadic = true;
    let loaded = f.load(root, f.word);
    f.ret(loaded);
    f.check(
        0,
        &[Run {
            mutation: Some((0, 91)),
            ..Run::default()
        }],
    );
}

#[test]
fn opaque_blocks_forwarding_even_with_an_earlier_no_write_call() {
    let mut f = Fixture::new();
    let root = f.cast(f.root, f.word);
    let seven = f.graph.add_constant(7, f.word);
    f.store(root, seven);
    let call = f.call(root, ArgPermission::None);
    set_attrs(&mut f, call, MemoryEffects::ReadOnly);
    f.fixed(Opcode::Opaque, &[seven], 0, Some(f.word));
    let loaded = f.load(root, f.word);
    f.ret(loaded);
    f.check(0, &[Run::default()]);
}

#[test]
fn readonly_call_still_prevents_dead_store_elimination() {
    let mut f = Fixture::new();
    let root = f.cast(f.root, f.word);
    let seven = f.graph.add_constant(7, f.word);
    // This retained write validates the following raw-address store, so DSE
    // has an independent access proof and must stop specifically at the call.
    f.store(root, seven);
    f.store(root, seven);
    let call = f.call(root, ArgPermission::None);
    set_attrs(&mut f, call, MemoryEffects::ReadOnly);
    let ninety = f.graph.add_constant(90, f.word);
    f.store(root, ninety);
    f.ret(ninety);
    verify_graph(&mut f.graph).unwrap();
    let original = f.graph.clone();
    let stats = crate::transforms::memory::dse::run(&mut f.graph);
    assert_eq!(stats.stores_removed, 0);
    assert_eq!(
        execute(&f.graph, &Run::default()),
        execute(&original, &Run::default())
    );
}

#[test]
fn a_nonreturning_call_remains_executed_after_forwarding() {
    struct Diverges(usize);
    impl SemanticCallHandler for Diverges {
        fn call(&mut self, _: &str, _: &[Value]) -> Result<Value, InterpretError> {
            self.0 += 1;
            Err(InterpretError::OutOfFuel)
        }
    }
    let mut f = Fixture::new();
    let root = f.cast(f.root, f.word);
    let seven = f.graph.add_constant(7, f.word);
    f.store(root, seven);
    let call = f.call(root, ArgPermission::None);
    set_attrs(&mut f, call, MemoryEffects::None);
    f.graph.callee_attrs.get_mut("observe").unwrap().noreturn = true;
    let loaded = f.load(root, f.word);
    f.ret(loaded);
    verify_graph(&mut f.graph).unwrap();
    let original = f.graph.clone();
    assert_eq!(run(&mut f.graph).loads_forwarded, 1);
    for graph in [&original, &f.graph] {
        let mut memory = SemanticMemory::new();
        let address = memory.allocate(64, 8, true).unwrap();
        let mut calls = Diverges(0);
        let result = Interpreter::new(graph).run_values_in_memory(
            &[Value::Scalar(address), Value::Scalar(address + 32)],
            &mut calls,
            &mut memory,
        );
        assert_eq!(result, Err(InterpretError::OutOfFuel));
        assert_eq!(calls.0, 1);
    }
}
