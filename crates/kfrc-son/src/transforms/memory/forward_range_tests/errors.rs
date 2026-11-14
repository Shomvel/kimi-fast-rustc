use super::*;

#[test]
fn removing_a_later_load_keeps_the_first_load_at_its_error_site() {
    let mut f = Fixture::new();
    let address = f.cast(f.root, f.boolean);
    f.load(address, f.boolean);
    let borrowed = f.reference(address, f.boolean, Mutability::Not);
    f.call(borrowed, frozen(1));
    let loaded = f.load(address, f.boolean);
    f.ret(loaded);
    f.check(1, &[Run::default()]);
    let execution = execute(&f.graph, &Run::default());
    assert!(matches!(
        execution.result,
        Err(InterpretError::UndefinedBehaviour(_))
    ));
    assert!(execution.calls.is_empty());
    assert_eq!(execution.bytes, vec![Some(0x5a); 64]);
}

#[test]
fn the_call_and_its_failure_remain_between_the_snapshot_and_later_load() {
    let mut f = Fixture::new();
    let address = f.field(f.root, f.pair, 1, f.word);
    let value = f.graph.add_constant(23, f.word);
    f.store(address, value);
    let borrowed = f.reference(f.root, f.pair, Mutability::Not);
    f.call(borrowed, frozen(16));
    let loaded = f.load(address, f.word);
    f.ret(loaded);
    f.check(
        1,
        &[Run {
            fail_call: true,
            ..Run::default()
        }],
    );
    let execution = execute(
        &f.graph,
        &Run {
            fail_call: true,
            ..Run::default()
        },
    );
    assert_eq!(
        execution.result,
        Err(InterpretError::UnknownCallee("observe failed".into()))
    );
    assert_eq!(execution.calls.len(), 1);
    assert_eq!(execution.calls[0], execution.bytes);
    assert_eq!(execution.bytes[8..16], 23u64.to_le_bytes().map(Some));
}

#[test]
fn a_failing_observation_after_the_call_is_not_moved_or_erased() {
    let mut f = Fixture::new();
    let address = f.field(f.root, f.pair, 1, f.word);
    let value = f.graph.add_constant(31, f.word);
    f.store(address, value);
    let borrowed = f.reference(f.root, f.pair, Mutability::Not);
    f.call(borrowed, frozen(16));
    let poison = f.graph.add_node(
        Opcode::Undefined,
        NodeId::NONE,
        &[],
        0,
        ValueType::Rust(f.word),
    );
    let observation = f.fixed(Opcode::RequireDefined, &[poison], 0, None).0;
    let loaded = f.load(address, f.word);
    f.ret(loaded);
    f.check(0, &[Run::default()]);
    let execution = execute(&f.graph, &Run::default());
    assert_eq!(
        execution.result,
        Err(InterpretError::PoisonObserved(observation))
    );
    assert_eq!(execution.calls.len(), 1);
    assert_eq!(execution.calls[0], execution.bytes);
}
