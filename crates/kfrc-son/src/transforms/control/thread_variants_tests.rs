use super::*;
use crate::{CallTarget, InterpretError, Interpreter, SemanticOutcome};
use kfrc_body::{FunctionAbi, IntegerWidth, PassMode, RecordedTypes, SemanticAggregate, TyId};

struct Fixture {
    graph: FunctionGraph,
    word: TyId,
    input: NodeId,
    region: NodeId,
    effect: NodeId,
    value: NodeId,
    otherwise: NodeId,
    right_effect: NodeId,
    right_variant: NodeId,
}

fn projection(graph: &mut FunctionGraph, source: NodeId, slot: u32, ty: ValueType) -> NodeId {
    graph.add_node(Opcode::Projection, NodeId::NONE, &[source], slot, ty)
}

fn call(
    graph: &mut FunctionGraph,
    control: NodeId,
    effect: NodeId,
    value: NodeId,
    word: TyId,
    name: &str,
) -> (NodeId, NodeId, NodeId) {
    let target = graph.add_callee(CallTarget {
        symbol: name.into(),
        abi: FunctionAbi {
            args: vec![PassMode::Direct],
            ret: PassMode::Direct,
            c_variadic: false,
        },
    });
    let node = graph.add_node(
        Opcode::Call,
        control,
        &[effect, value],
        target,
        ValueType::Tuple(vec![
            ValueType::Control,
            ValueType::Effect,
            ValueType::Rust(word),
        ]),
    );
    let control = projection(graph, node, 0, ValueType::Control);
    let effect = projection(graph, node, 1, ValueType::Effect);
    let value = projection(graph, node, 2, ValueType::Rust(word));
    (control, effect, value)
}

fn fixture() -> Fixture {
    let mut types = RecordedTypes::new();
    let word = types.intern(
        TypeShape::Int {
            width: IntegerWidth::W64,
            signed: false,
        },
        None,
    );
    let boolean = types.intern(TypeShape::Bool, None);
    let enumeration = types.intern(
        TypeShape::Adt {
            name: "ControlFlow".into(),
        },
        None,
    );
    types.record_semantic_aggregate(
        enumeration,
        SemanticAggregate {
            fields: vec![],
            variants: vec![vec![word], vec![word]],
        },
    );
    types.record_semantic_enum(enumeration, vec![0, 1]);
    let mut graph = FunctionGraph::new(
        "anchored_phi",
        FunctionAbi {
            args: vec![PassMode::Direct],
            ret: PassMode::Direct,
            c_variadic: false,
        },
        types,
    );
    graph.level = crate::GraphLevel::Specialized;
    let start = graph.add_node(
        Opcode::Start,
        NodeId::NONE,
        &[],
        0,
        ValueType::Tuple(vec![
            ValueType::Control,
            ValueType::Effect,
            ValueType::Rust(word),
        ]),
    );
    graph.start = start;
    let control = projection(&mut graph, start, 0, ValueType::Control);
    let effect = projection(&mut graph, start, 1, ValueType::Effect);
    let input = projection(&mut graph, start, 2, ValueType::Rust(word));
    let zero = graph.add_constant(0, word);
    let condition = graph.add_node(
        Opcode::Compare,
        NodeId::NONE,
        &[input, zero],
        ComparePredicate::Equal as u32,
        ValueType::Rust(boolean),
    );
    let branch = graph.add_node(
        Opcode::If,
        control,
        &[condition],
        0,
        ValueType::Tuple(vec![ValueType::Control; 2]),
    );
    let mut incoming = Vec::new();
    let mut values = Vec::new();
    let mut effects = Vec::new();
    for variant in 0..2 {
        let control = projection(&mut graph, branch, variant, ValueType::Control);
        let (control, effect, value) = call(
            &mut graph,
            control,
            effect,
            input,
            word,
            if variant == 0 { "left" } else { "right" },
        );
        incoming.push(graph.add_node(Opcode::Jump, control, &[], 0, ValueType::Control));
        effects.push(effect);
        values.push(graph.add_node(
            Opcode::MakeVariant,
            NodeId::NONE,
            &[value],
            variant,
            ValueType::Rust(enumeration),
        ));
    }
    let region = graph.add_node(
        Opcode::Region,
        NodeId::NONE,
        &incoming,
        0,
        ValueType::Control,
    );
    let value = graph.add_node(
        Opcode::Phi,
        NodeId::NONE,
        &[region, values[0], values[1]],
        0,
        ValueType::Rust(enumeration),
    );
    // Exact shape produced by inlining in semver::Version::from_str.
    let effect = graph.add_node(
        Opcode::Phi,
        region,
        &[region, effects[0], effects[1]],
        0,
        ValueType::Effect,
    );
    graph.variant_switches.push(vec![0, 1]);
    let switch = graph.add_node(
        Opcode::VariantSwitch,
        region,
        &[value],
        0,
        ValueType::Tuple(vec![ValueType::Control; 3]),
    );
    let mut otherwise = NodeId::NONE;
    let mut right_effect = NodeId::NONE;
    for variant in 0..2 {
        let control = projection(&mut graph, switch, variant, ValueType::Control);
        let witness = graph.add_node(
            Opcode::VariantWitness,
            control,
            &[value, control],
            variant,
            ValueType::Witness,
        );
        let field = graph.intern_variant_field(variant, 0);
        let field = graph.add_node(
            Opcode::VariantField,
            NodeId::NONE,
            &[value, witness],
            field,
            ValueType::Rust(word),
        );
        let (control, effect, result) = call(
            &mut graph,
            control,
            effect,
            field,
            word,
            if variant == 0 { "continue" } else { "break" },
        );
        let ret = graph.add_node(
            Opcode::Return,
            control,
            &[effect, result],
            0,
            ValueType::Control,
        );
        if variant == 1 {
            otherwise = ret;
            right_effect = effect;
        }
    }
    let default = projection(&mut graph, switch, 2, ValueType::Control);
    graph.add_node(Opcode::Unreachable, default, &[], 0, ValueType::Control);
    Fixture {
        graph,
        word,
        input,
        region,
        effect,
        value,
        otherwise,
        right_effect,
        right_variant: values[1],
    }
}

fn execute(
    graph: &FunctionGraph,
    input: u64,
) -> (Result<SemanticOutcome, InterpretError>, Vec<String>) {
    let mut trace = Vec::new();
    let mut interpreter = Interpreter::new(graph);
    interpreter.fuel = 100;
    let outcome = interpreter.run_values(
        &[crate::Value::Scalar(input)],
        &mut |name: &str, args: &[crate::Value]| {
            trace.push(format!("{name}:{args:?}"));
            Ok(args[0].clone())
        },
    );
    (outcome, trace)
}

#[test]
fn anchored_effect_phi_threads_without_changing_payloads_or_calls() {
    let mut f = fixture();
    crate::verify_graph(&mut f.graph).unwrap();
    let expected = [0, 1, 17, u64::MAX].map(|input| execute(&f.graph, input));
    let stats = super::run(&mut f.graph).unwrap();
    assert_eq!(stats.regions_threaded, 1);
    assert_eq!(stats.branches_removed, 1);
    crate::verify_graph(&mut f.graph).unwrap();
    for (input, expected) in [0, 1, 17, u64::MAX].into_iter().zip(expected) {
        assert_eq!(execute(&f.graph, input), expected);
    }
    assert_eq!(
        f.graph
            .ids()
            .filter(|&node| f.graph.op(node) == Opcode::Call)
            .count(),
        4
    );
    assert!(
        !f.graph
            .ids()
            .any(|node| f.graph.op(node) == Opcode::VariantSwitch)
    );
}

#[test]
fn accepting_phi_anchors_does_not_accept_an_unrelated_trapping_control_user() {
    let mut f = fixture();
    let zero = f.graph.add_constant(0, f.word);
    f.graph.add_node(
        Opcode::DivUnsigned,
        f.region,
        &[f.input, zero],
        0,
        ValueType::Rust(f.word),
    );
    crate::verify_graph(&mut f.graph).unwrap();
    let expected = [0, 1].map(|input| execute(&f.graph, input));
    assert_eq!(super::run(&mut f.graph).unwrap().regions_threaded, 0);
    crate::verify_graph(&mut f.graph).unwrap();
    for (input, expected) in [0, 1].into_iter().zip(expected) {
        assert_eq!(execute(&f.graph, input), expected);
    }
}

#[test]
fn accepting_phi_anchors_keeps_loop_carried_regions_unthreaded() {
    let mut f = fixture();
    let control = f.graph.control(f.otherwise);
    f.graph.remove_node(f.otherwise);
    let backedge = f
        .graph
        .add_node(Opcode::Jump, control, &[], 0, ValueType::Control);
    let mut inputs = f.graph.inputs(f.region).to_vec();
    inputs.push(backedge);
    f.graph.set_inputs(f.region, &inputs);
    let mut inputs = f.graph.inputs(f.value).to_vec();
    // Reuse the original input on the backedge; no effectful value is cloned.
    let ty = f.graph.value_type(f.right_variant).clone();
    let incoming = f
        .graph
        .add_node(Opcode::MakeVariant, NodeId::NONE, &[f.input], 1, ty);
    inputs.push(incoming);
    f.graph.set_inputs(f.value, &inputs);
    let mut inputs = f.graph.inputs(f.effect).to_vec();
    inputs.push(f.right_effect);
    f.graph.set_inputs(f.effect, &inputs);
    crate::verify_graph(&mut f.graph).unwrap();
    let expected = [0, 1].map(|input| execute(&f.graph, input));
    assert_eq!(super::run(&mut f.graph).unwrap().regions_threaded, 0);
    crate::verify_graph(&mut f.graph).unwrap();
    for (input, expected) in [0, 1].into_iter().zip(expected) {
        assert_eq!(execute(&f.graph, input), expected);
    }
}

struct SpineFixture {
    regions: Vec<NodeId>,
    effects: Vec<NodeId>,
    values: Vec<NodeId>,
    switch: NodeId,
}

fn forward_uses(graph: &mut FunctionGraph, uses: &[crate::analysis::uses::Use], value: NodeId) {
    for usage in uses {
        let UseKind::Operand(slot) = usage.kind else {
            panic!("fixture expected operand uses");
        };
        graph.set_input(usage.user, slot, value);
    }
}

fn add_spine(f: &mut Fixture, length: usize) -> SpineFixture {
    let switch = f
        .graph
        .ids()
        .find(|&node| f.graph.op(node) == Opcode::VariantSwitch)
        .unwrap();
    let mut region = f.region;
    let mut effect = f.effect;
    let mut value = f.value;
    let mut spine = SpineFixture {
        regions: vec![],
        effects: vec![],
        values: vec![],
        switch,
    };
    for _ in 0..length {
        let uses = Uses::new(&f.graph);
        let effect_uses = uses.of(effect).to_vec();
        let value_uses = uses.of(value).to_vec();
        let jump = f
            .graph
            .add_node(Opcode::Jump, region, &[], 0, ValueType::Control);
        region = f
            .graph
            .add_node(Opcode::Region, NodeId::NONE, &[jump], 0, ValueType::Control);
        effect = f
            .graph
            .add_node(Opcode::Phi, region, &[region, effect], 0, ValueType::Effect);
        value = f.graph.add_node(
            Opcode::Phi,
            NodeId::NONE,
            &[region, value],
            0,
            f.graph.value_type(value).clone(),
        );
        forward_uses(&mut f.graph, &effect_uses, effect);
        forward_uses(&mut f.graph, &value_uses, value);
        spine.regions.push(region);
        spine.effects.push(effect);
        spine.values.push(value);
    }
    f.graph.set_control(switch, region);
    spine
}

#[test]
fn transparent_spine_preserves_all_exits_payloads_and_effects() {
    for length in [1, 3] {
        let mut f = fixture();
        let spine = add_spine(&mut f, length);
        crate::verify_graph(&mut f.graph).unwrap();
        let expected = [0, 1, 17, u64::MAX].map(|input| execute(&f.graph, input));
        assert!(expected.iter().all(|(outcome, _)| outcome.is_ok()));
        let stats = super::run(&mut f.graph).unwrap();
        assert_eq!(stats.control_spines_collapsed, 1);
        assert_eq!(stats.regions_threaded, 1);
        assert!(f.graph.is_dead(spine.switch));
        assert!(spine.regions.iter().all(|&region| f.graph.is_dead(region)));
        crate::verify_graph(&mut f.graph).unwrap();
        for (input, expected) in [0, 1, 17, u64::MAX].into_iter().zip(expected) {
            assert_eq!(execute(&f.graph, input), expected);
        }
    }
}

#[test]
fn spine_collapse_reports_dirty_users_even_when_variant_threading_is_rejected() {
    let mut f = fixture();
    let unknown = f.graph.add_node(
        Opcode::Undefined,
        NodeId::NONE,
        &[],
        0,
        f.graph.value_type(f.value).clone(),
    );
    f.graph.set_input(f.value, 2, unknown);
    let spine = add_spine(&mut f, 1);
    crate::verify_graph(&mut f.graph).unwrap();
    let expected = [0, 1].map(|input| execute(&f.graph, input));
    let stats = super::run(&mut f.graph).unwrap();
    assert_eq!(stats.control_spines_collapsed, 1);
    assert_eq!(stats.regions_threaded, 0);
    assert!(stats.dirty_nodes.contains(&spine.switch));
    assert_eq!(f.graph.control(spine.switch), f.region);
    crate::verify_graph(&mut f.graph).unwrap();
    for (input, expected) in [0, 1].into_iter().zip(expected) {
        assert_eq!(execute(&f.graph, input), expected);
    }
}

#[test]
fn transparent_spine_threads_values_consumed_on_a_loop_phi_edge() {
    let mut f = fixture();
    let first_if = f
        .graph
        .ids()
        .find(|&node| f.graph.op(node) == Opcode::If)
        .unwrap();
    let initial_control = f.graph.control(first_if);
    let initial_effect = f
        .graph
        .ids()
        .find(|&node| {
            f.graph.op(node) == Opcode::Projection
                && f.graph.input(node, 0) == f.graph.start
                && f.graph.value_type(node) == &ValueType::Effect
        })
        .unwrap();
    let uses = Uses::new(&f.graph);
    let input_uses = uses.of(f.input).to_vec();
    let effect_uses = uses.of(initial_effect).to_vec();
    let entry = f
        .graph
        .add_node(Opcode::Jump, initial_control, &[], 0, ValueType::Control);
    let backedge = f.graph.add_node(
        Opcode::Jump,
        f.graph.control(f.otherwise),
        &[],
        0,
        ValueType::Control,
    );
    f.graph.remove_node(f.otherwise);
    let header = f.graph.add_node(
        Opcode::Region,
        NodeId::NONE,
        &[entry, backedge],
        0,
        ValueType::Control,
    );
    let induction = f.graph.add_node(
        Opcode::Phi,
        NodeId::NONE,
        &[header, f.input, f.input],
        0,
        ValueType::Rust(f.word),
    );
    let effect = f.graph.add_node(
        Opcode::Phi,
        header,
        &[header, initial_effect, f.right_effect],
        0,
        ValueType::Effect,
    );
    forward_uses(&mut f.graph, &input_uses, induction);
    forward_uses(&mut f.graph, &effect_uses, effect);
    f.graph.set_control(first_if, header);
    let one = f.graph.add_constant(1, f.word);
    let next = f.graph.add_node(
        Opcode::Sub,
        NodeId::NONE,
        &[induction, one],
        0,
        ValueType::Rust(f.word),
    );
    let merged = f.graph.add_node(
        Opcode::Phi,
        NodeId::NONE,
        &[f.region, induction, next],
        0,
        ValueType::Rust(f.word),
    );
    f.graph.set_input(induction, 2, merged);
    add_spine(&mut f, 1);
    crate::verify_graph(&mut f.graph).unwrap();
    let expected = [0, 1, 3].map(|input| execute(&f.graph, input));
    assert!(expected.iter().all(|(outcome, _)| outcome.is_ok()));
    let stats = super::run(&mut f.graph).unwrap();
    assert_eq!(stats.control_spines_collapsed, 1);
    assert_eq!(stats.regions_threaded, 1);
    assert!(f.graph.is_dead(merged));
    assert_eq!(f.graph.input(induction, 2), next);
    crate::verify_graph(&mut f.graph).unwrap();
    for (input, expected) in [0, 1, 3].into_iter().zip(expected) {
        assert_eq!(execute(&f.graph, input), expected);
    }
}

#[test]
fn a_trapping_control_user_makes_the_whole_spine_nontransparent() {
    let mut f = fixture();
    let spine = add_spine(&mut f, 3);
    let zero = f.graph.add_constant(0, f.word);
    f.graph.add_node(
        Opcode::DivUnsigned,
        spine.regions[1],
        &[f.input, zero],
        0,
        ValueType::Rust(f.word),
    );
    crate::verify_graph(&mut f.graph).unwrap();
    let expected = [0, 1].map(|input| execute(&f.graph, input));
    let stats = super::run(&mut f.graph).unwrap();
    assert_eq!(stats.control_spines_collapsed, 0);
    assert_eq!(stats.regions_threaded, 0);
    assert!(spine.regions.iter().all(|&region| !f.graph.is_dead(region)));
    crate::verify_graph(&mut f.graph).unwrap();
    for (input, expected) in [0, 1].into_iter().zip(expected) {
        assert_eq!(execute(&f.graph, input), expected);
    }
}

#[test]
fn an_effectful_call_is_not_bypassed_by_spine_collapse() {
    let mut f = fixture();
    let spine = add_spine(&mut f, 1);
    let effect_uses = Uses::new(&f.graph).of(spine.effects[0]).to_vec();
    let (control, effect, _) = call(
        &mut f.graph,
        spine.regions[0],
        spine.effects[0],
        f.input,
        f.word,
        "barrier",
    );
    forward_uses(&mut f.graph, &effect_uses, effect);
    f.graph.set_control(spine.switch, control);
    crate::verify_graph(&mut f.graph).unwrap();
    let expected = [0, 1].map(|input| execute(&f.graph, input));
    assert!(
        expected
            .iter()
            .all(|(_, trace)| trace.iter().any(|entry| entry.starts_with("barrier:")))
    );
    let stats = super::run(&mut f.graph).unwrap();
    assert_eq!(stats.control_spines_collapsed, 0);
    assert_eq!(stats.regions_threaded, 0);
    crate::verify_graph(&mut f.graph).unwrap();
    for (input, expected) in [0, 1].into_iter().zip(expected) {
        assert_eq!(execute(&f.graph, input), expected);
    }
}

#[test]
fn a_multi_entry_loop_region_is_not_a_transparent_wrapper() {
    let mut f = fixture();
    let spine = add_spine(&mut f, 1);
    let backedge = f.graph.add_node(
        Opcode::Jump,
        f.graph.control(f.otherwise),
        &[],
        0,
        ValueType::Control,
    );
    f.graph.remove_node(f.otherwise);
    let region = spine.regions[0];
    let entry = f.graph.input(region, 0);
    f.graph.set_inputs(region, &[entry, backedge]);
    f.graph
        .set_inputs(spine.effects[0], &[region, f.effect, f.right_effect]);
    // The original right-arm payload does not structurally dominate this
    // backedge: it has already crossed the outer merge. Carry its merged value.
    f.graph
        .set_inputs(spine.values[0], &[region, f.value, f.value]);
    crate::verify_graph(&mut f.graph).unwrap();
    let expected = [0, 1].map(|input| execute(&f.graph, input));
    let stats = super::run(&mut f.graph).unwrap();
    assert_eq!(stats.control_spines_collapsed, 0);
    assert_eq!(stats.regions_threaded, 0);
    crate::verify_graph(&mut f.graph).unwrap();
    for (input, expected) in [0, 1].into_iter().zip(expected) {
        assert_eq!(execute(&f.graph, input), expected);
    }
}
