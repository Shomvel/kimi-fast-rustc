use super::*;

#[test]
fn refreshed_loop_phi_equality_stays_local_to_each_iteration() {
    let mut f = Fixture::new(IntegerWidth::W64, false);
    let entry = f.jump(f.control);
    let header = f.graph.add_node(
        Opcode::Region,
        NodeId::NONE,
        &[entry, entry],
        0,
        ValueType::Control,
    );
    let value = f.graph.add_node(
        Opcode::Phi,
        NodeId::NONE,
        &[header, f.args[0], f.args[1]],
        0,
        ValueType::Rust(f.word),
    );
    let effect = f.graph.add_node(
        Opcode::Phi,
        NodeId::NONE,
        &[header, f.effect, f.effect],
        0,
        ValueType::Effect,
    );
    let constant = f.graph.add_constant(5, f.word);
    f.control = header;
    f.effect = effect;
    let condition = f.compare(value, constant, P::Equal);
    let [yes, no] = f.branch(condition);
    f.ret(no, effect, value);
    f.control = yes;
    let (call, _) = f.call("iteration", &[value]);
    let back = f.jump(f.control);
    f.graph.set_input(header, 1, back);
    f.graph.set_input(effect, 2, f.effect);
    let before = f.apply(1);
    assert_eq!(f.graph.input(call, 1), constant);
    for input in [0, 5] {
        assert_eq!(
            execute(&f.graph, [input, 9, 0]),
            execute(&before, [input, 9, 0])
        );
    }
    assert_eq!(
        execute(&f.graph, [5, 9, 0]).1,
        vec![("iteration".into(), vec![5])]
    );
}

#[test]
fn older_proof_cannot_cross_a_redefined_loop_phi() {
    let mut f = Fixture::new(IntegerWidth::W64, false);
    let constant = f.graph.add_constant(5, f.word);
    let condition = f.compare(f.args[0], constant, P::Equal);
    let [proof, exit] = f.branch(condition);
    f.ret(exit, f.effect, f.args[0]);
    f.control = proof;
    let entry = f.jump(proof);
    let header = f.graph.add_node(
        Opcode::Region,
        NodeId::NONE,
        &[entry, entry],
        0,
        ValueType::Control,
    );
    let value = f.graph.add_node(
        Opcode::Phi,
        NodeId::NONE,
        &[header, f.args[0], f.args[1]],
        0,
        ValueType::Rust(f.word),
    );
    let zero = f.graph.add_constant(0, f.word);
    let one = f.graph.add_constant(1, f.word);
    let iteration = f.graph.add_node(
        Opcode::Phi,
        NodeId::NONE,
        &[header, zero, one],
        0,
        ValueType::Rust(f.word),
    );
    let incoming_effect = f.effect;
    let effect = f.graph.add_node(
        Opcode::Phi,
        NodeId::NONE,
        &[header, incoming_effect, incoming_effect],
        0,
        ValueType::Effect,
    );
    f.control = header;
    f.effect = effect;
    let (call, result) = f.call("carried", &[value, f.args[0]]);
    let again = f.compare(iteration, zero, P::Equal);
    let [yes, no] = f.branch(again);
    let back = f.jump(yes);
    f.graph.set_input(header, 1, back);
    f.graph.set_input(effect, 2, f.effect);
    f.ret(no, f.effect, result);
    crate::verify_graph(&mut f.graph).unwrap();
    let cfg = ControlFlow::new(&f.graph);
    let dom = Dominators::new(&cfg);
    let mut facts = PredicateFacts::new(&f.graph, &cfg, &dom);
    assert_eq!(
        facts.integer_constant_at(value, proof, f.graph.control(call)),
        None
    );
    let before = f.apply(1);
    assert_eq!(f.graph.input(call, 1), value);
    assert_eq!(f.graph.input(call, 2), constant);
    for input in [0, 5] {
        assert_eq!(
            execute(&f.graph, [input, 9, 0]),
            execute(&before, [input, 9, 0])
        );
    }
    assert_eq!(
        execute(&f.graph, [5, 9, 0]).1,
        vec![
            ("carried".into(), vec![5, 5]),
            ("carried".into(), vec![9, 5])
        ]
    );
}

#[test]
fn exact_ssa_fact_rejects_a_dirty_dynamic_instance_and_accepts_refresh() {
    let mut f = Fixture::new(IntegerWidth::W64, false);
    let entry = f.jump(f.control);
    let header = f.graph.add_node(
        Opcode::Region,
        NodeId::NONE,
        &[entry],
        0,
        ValueType::Control,
    );
    let constant = f.graph.add_constant(5, f.word);
    // This is an adversarial fact query, deliberately outside valid SSA:
    // the outer proof reads a Phi anchored in an inner Region. Its original
    // bits cannot justify a later dynamic instance of that exact NodeId.
    let value = f.graph.add_node(
        Opcode::Phi,
        NodeId::NONE,
        &[header, f.args[0]],
        0,
        ValueType::Rust(f.word),
    );
    f.control = header;
    let condition = f.compare(value, constant, P::Equal);
    let [proof, exit] = f.branch(condition);
    let entered = f.jump(proof);
    let inner = f.graph.add_node(
        Opcode::Region,
        NodeId::NONE,
        &[entered, entered],
        0,
        ValueType::Control,
    );
    f.control = inner;
    let [again, refresh] = f.branch(f.args[2]);
    let back = f.jump(again);
    let refreshed = f.jump(refresh);
    f.graph.set_input(inner, 1, back);
    f.graph.set_inputs(header, &[entry, refreshed]);
    f.graph.set_inputs(value, &[inner, f.args[0], f.args[1]]);
    f.ret(exit, f.effect, f.args[0]);
    let cfg = ControlFlow::new(&f.graph);
    let dom = Dominators::new(&cfg);
    assert!(dom.dominates(&cfg, proof, back));
    let mut facts = PredicateFacts::new(&f.graph, &cfg, &dom);
    assert_eq!(facts.integer_constant_at(value, proof, back), None);

    // Move the same Phi to the outer header. Every path which changes it now
    // traverses proof again before back, so the refreshed instance is valid.
    f.graph.set_input(value, 0, header);
    let cfg = ControlFlow::new(&f.graph);
    let dom = Dominators::new(&cfg);
    let mut facts = PredicateFacts::new(&f.graph, &cfg, &dom);
    assert_eq!(
        facts.integer_constant_at(value, proof, back),
        Some(constant)
    );
}
