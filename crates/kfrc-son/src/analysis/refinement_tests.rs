use super::*;
use crate::{ValueType, verify_graph};
use kfrc_body::{FunctionAbi, PassMode, RecordedTypes, TypeShape};

struct Fixture {
    graph: FunctionGraph,
    control: NodeId,
    effect: NodeId,
    predicate: NodeId,
}
impl Fixture {
    fn new() -> Self {
        let mut types = RecordedTypes::new();
        let boolean = types.intern(TypeShape::Bool, None);
        let mut graph = FunctionGraph::new(
            "scoped_facts",
            FunctionAbi {
                args: vec![PassMode::Direct],
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
        let predicate = graph.add_node(
            Opcode::Projection,
            NodeId::NONE,
            &[start],
            2,
            ValueType::Rust(boolean),
        );
        Self {
            graph,
            control,
            effect,
            predicate,
        }
    }
    fn branch(&mut self, control: NodeId, predicate: NodeId) -> (NodeId, NodeId) {
        let branch = self.graph.add_node(
            Opcode::If,
            control,
            &[predicate],
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
    fn jump(&mut self, control: NodeId) -> NodeId {
        self.graph
            .add_node(Opcode::Jump, control, &[], 0, ValueType::Control)
    }
    fn ret(&mut self, control: NodeId) -> NodeId {
        self.graph.add_node(
            Opcode::Return,
            control,
            &[self.effect, self.predicate],
            0,
            ValueType::Control,
        )
    }
}

#[test]
fn branch_facts_record_both_truth_values_and_do_not_escape_a_merge() {
    let mut f = Fixture::new();
    let (yes, no) = f.branch(f.control, f.predicate);
    let left = f.jump(yes);
    let right = f.jump(no);
    let merge = f.graph.add_node(
        Opcode::Region,
        NodeId::NONE,
        &[left, right],
        0,
        ValueType::Control,
    );
    f.ret(merge);
    verify_graph(&mut f.graph).unwrap();
    let cfg = ControlFlow::new(&f.graph);
    let dom = Dominators::new(&cfg);
    let mut facts = PredicateFacts::new(&f.graph, &cfg, &dom);
    for (proof, at, truth) in [(yes, left, true), (no, right, false)] {
        assert_eq!(
            facts.predicate_at(proof, at),
            Some(ScopedPredicate {
                predicate: f.predicate,
                truth,
                proof,
                origin: PredicateOrigin::Branch,
            })
        );
        assert_eq!(facts.predicate_at(proof, merge), None);
    }
    assert_eq!(facts.predicate_at(yes, no), None);
    let visits = facts.stats().node_visits;
    assert!(facts.predicate_at(yes, left).is_some());
    assert_eq!(
        facts.stats().node_visits,
        visits,
        "cached queries must not rescan CFG"
    );
}

#[test]
fn assumption_fact_begins_at_its_normal_continuation() {
    let mut f = Fixture::new();
    let assume = f.graph.add_node(
        Opcode::Assume,
        f.control,
        &[f.effect, f.predicate],
        0,
        ValueType::Tuple(vec![ValueType::Control, ValueType::Effect]),
    );
    let next = f.graph.add_node(
        Opcode::Projection,
        NodeId::NONE,
        &[assume],
        0,
        ValueType::Control,
    );
    f.effect = f.graph.add_node(
        Opcode::Projection,
        NodeId::NONE,
        &[assume],
        1,
        ValueType::Effect,
    );
    let ret = f.ret(next);
    verify_graph(&mut f.graph).unwrap();
    let cfg = ControlFlow::new(&f.graph);
    let dom = Dominators::new(&cfg);
    let mut facts = PredicateFacts::new(&f.graph, &cfg, &dom);
    assert_eq!(
        facts.predicate_at(next, ret),
        Some(ScopedPredicate {
            predicate: f.predicate,
            truth: true,
            proof: next,
            origin: PredicateOrigin::Assumption,
        })
    );
    assert_eq!(facts.predicate_at(next, assume), None);
    assert_eq!(facts.predicate_at(f.effect, ret), None);
}

#[test]
fn loop_scan_rejects_changed_instances_and_accepts_proof_refresh() {
    let mut f = Fixture::new();
    let entry = f.jump(f.control);
    let header = f.graph.add_node(
        Opcode::Region,
        NodeId::NONE,
        &[entry],
        0,
        ValueType::Control,
    );
    let (proof, exit) = f.branch(header, f.predicate);
    let use_site = f.jump(proof);
    let inner = f.graph.add_node(
        Opcode::Region,
        NodeId::NONE,
        &[use_site],
        0,
        ValueType::Control,
    );
    let (again, outer) = f.branch(inner, f.predicate);
    let back = f.jump(again);
    let refresh = f.jump(outer);
    f.graph.set_inputs(inner, &[use_site, back]);
    f.graph.set_inputs(header, &[entry, refresh]);
    f.ret(exit);
    verify_graph(&mut f.graph).unwrap();
    let cfg = ControlFlow::new(&f.graph);
    let mut stats = WitnessStats::default();
    // Re-entering an inner dynamic definition without passing proof again is stale.
    assert!(!unchanged_until(
        &cfg,
        true,
        proof,
        back,
        &[inner],
        &mut stats
    ));
    // Re-entering the outer definition always traverses proof again before use.
    assert!(unchanged_until(
        &cfg,
        true,
        proof,
        back,
        &[header],
        &mut stats
    ));
    assert!(stats.node_visits > 0);
    let dom = Dominators::new(&cfg);
    let mut facts = PredicateFacts::new(&f.graph, &cfg, &dom);
    assert!(
        facts.predicate_at(proof, back).is_some(),
        "loop-invariant input stays valid"
    );
    assert!(
        facts.stats().node_visits < 200,
        "bounded CFG, not path enumeration"
    );
}

#[test]
fn variant_predicate_requires_equality_truth_and_exact_source() {
    use kfrc_body::{IntegerWidth, SemanticAggregate};
    let mut f = Fixture::new();
    let word = f.graph.oracle.intern(
        TypeShape::Int {
            width: IntegerWidth::W64,
            signed: false,
        },
        None,
    );
    let enumeration = f.graph.oracle.intern(
        TypeShape::Adt {
            name: "Choice".into(),
        },
        None,
    );
    f.graph.oracle.record_semantic_aggregate(
        enumeration,
        SemanticAggregate {
            fields: vec![],
            variants: vec![vec![], vec![word]],
        },
    );
    f.graph.oracle.record_semantic_enum(enumeration, vec![0, 1]);
    let enum_value = f.graph.add_node(
        Opcode::Undefined,
        NodeId::NONE,
        &[],
        0,
        ValueType::Rust(enumeration),
    );
    let other = f.graph.add_node(
        Opcode::Undefined,
        NodeId::NONE,
        &[],
        0,
        ValueType::Rust(enumeration),
    );
    let tag = f.graph.add_node(
        Opcode::VariantOf,
        NodeId::NONE,
        &[enum_value],
        0,
        ValueType::Rust(word),
    );
    let one = f.graph.add_constant(1, word);
    let boolean = f.graph.rust_type(f.predicate).unwrap();
    let compare = f.graph.add_node(
        Opcode::Compare,
        NodeId::NONE,
        &[tag, one],
        ComparePredicate::NotEqual as u32,
        ValueType::Rust(boolean),
    );
    let (yes, no) = f.branch(f.control, compare);
    let witness = f.graph.add_node(
        Opcode::VariantWitness,
        no,
        &[enum_value, no],
        1,
        ValueType::Witness,
    );
    let payload = f.graph.intern_variant_field(1, 0);
    let field = f.graph.add_node(
        Opcode::VariantField,
        NodeId::NONE,
        &[enum_value, witness],
        payload,
        ValueType::Rust(word),
    );
    f.graph.add_node(
        Opcode::Return,
        no,
        &[f.effect, field],
        0,
        ValueType::Control,
    );
    f.ret(yes);
    verify_graph(&mut f.graph).unwrap();
    let cfg = ControlFlow::new(&f.graph);
    let dom = Dominators::new(&cfg);
    let mut facts = PredicateFacts::new(&f.graph, &cfg, &dom);
    let false_fact = facts.predicate_at(no, no).unwrap();
    assert_eq!(predicate_variant(&f.graph, enum_value, false_fact), Some(1));
    assert_eq!(predicate_variant(&f.graph, other, false_fact), None);
    assert_eq!(
        predicate_variant(&f.graph, enum_value, facts.predicate_at(yes, yes).unwrap()),
        None
    );
    // Incorrect proof is a verifier error even when the source program would
    // never reach this branch (the test input here is poison).
    f.graph.set_inputs(witness, &[enum_value, yes]);
    assert!(verify_graph(&mut f.graph).is_err());
}
