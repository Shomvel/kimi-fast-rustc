//! Loop facts come from control dominance, not node order or value Phi shape.
use kfrc_body::{FunctionAbi, IntegerWidth, PassMode, RecordedTypes, SizeAlign, TyId, TypeShape};
use kfrc_son::{
    ComparePredicate, FunctionGraph, GraphLevel, InterpretError, Interpreter, NodeId, Opcode,
    SemanticOutcome, Value, ValueType,
    analysis::{
        cfg::ControlFlow,
        dominators::Dominators,
        loops::{ControlEdge, LoopInfo},
    },
    verify_graph,
};

struct Fixture {
    graph: FunctionGraph,
    entry: NodeId,
    effect: NodeId,
    input: NodeId,
    predicate: NodeId,
    word: TyId,
    boolean: TyId,
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
        let mut graph = FunctionGraph::new(
            "loop_facts",
            FunctionAbi {
                args: vec![PassMode::Direct],
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
                ValueType::Rust(word),
            ]),
        );
        graph.start = start;
        let entry = graph.add_node(
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
        let input = graph.add_node(
            Opcode::Projection,
            NodeId::NONE,
            &[start],
            2,
            ValueType::Rust(word),
        );
        let zero = graph.add_constant(0, word);
        let predicate = graph.add_node(
            Opcode::Compare,
            NodeId::NONE,
            &[input, zero],
            ComparePredicate::NotEqual as u32,
            ValueType::Rust(boolean),
        );
        Self {
            graph,
            entry,
            effect,
            input,
            predicate,
            word,
            boolean,
        }
    }

    fn region(&mut self, incoming: &[NodeId]) -> NodeId {
        self.graph.add_node(
            Opcode::Region,
            NodeId::NONE,
            incoming,
            0,
            ValueType::Control,
        )
    }

    fn jump(&mut self, from: NodeId) -> NodeId {
        self.graph
            .add_node(Opcode::Jump, from, &[], 0, ValueType::Control)
    }

    fn branch(&mut self, from: NodeId, predicate: NodeId) -> (NodeId, NodeId) {
        let branch = self.graph.add_node(
            Opcode::If,
            from,
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

    fn ret(&mut self, control: NodeId, value: NodeId) {
        self.graph.add_node(
            Opcode::Return,
            control,
            &[self.effect, value],
            0,
            ValueType::Control,
        );
    }

    fn facts(&mut self) -> LoopInfo {
        verify_graph(&mut self.graph).unwrap();
        let before = self.graph.dump();
        let cfg = ControlFlow::new(&self.graph);
        let facts = LoopInfo::new(&cfg, &Dominators::new(&cfg));
        assert_eq!(
            before,
            self.graph.dump(),
            "read-only analysis mutated the graph"
        );
        facts
    }
}

#[test]
fn acyclic_diamond_and_unreachable_cycle_do_not_create_a_reachable_loop() {
    let mut f = Fixture::new();
    let (yes, no) = f.branch(f.entry, f.predicate);
    let a = f.jump(yes);
    let b = f.jump(no);
    let join = f.region(&[a, b]);
    f.ret(join, f.input);
    assert!(f.facts().loops().is_empty());
    // CFG construction deliberately tolerates unreachable control during edits.
    let dead_header = f.region(&[]);
    let dead_backedge = f.jump(dead_header);
    f.graph.set_inputs(dead_header, &[dead_backedge]);
    let cfg = ControlFlow::new(&f.graph);
    let facts = LoopInfo::new(&cfg, &Dominators::new(&cfg));
    assert!(facts.loops().is_empty());
    assert!(facts.irreducible_regions().is_empty());
    assert!(facts.loop_for(dead_header).is_none());
}

#[test]
fn canonical_loop_preserves_phi_edge_identity_and_execution() {
    let mut f = Fixture::new();
    let entry = f.jump(f.entry);
    let header = f.region(&[entry]);
    let zero = f.graph.add_constant(0, f.word);
    let one = f.graph.add_constant(1, f.word);
    let phi = f.graph.add_node(
        Opcode::Phi,
        NodeId::NONE,
        &[header, zero],
        0,
        ValueType::Rust(f.word),
    );
    let compare = f.graph.add_node(
        Opcode::Compare,
        NodeId::NONE,
        &[phi, f.input],
        ComparePredicate::UnsignedLess as u32,
        ValueType::Rust(f.boolean),
    );
    let (body, exit) = f.branch(header, compare);
    let next = f.graph.add_node(
        Opcode::Add,
        NodeId::NONE,
        &[phi, one],
        0,
        ValueType::Rust(f.word),
    );
    let latch = f.jump(body);
    f.graph.set_inputs(header, &[entry, latch]);
    f.graph.set_inputs(phi, &[header, zero, next]);
    f.ret(exit, phi);
    let facts = f.facts();
    let loop_ = facts.loop_by_header(header).unwrap();
    assert_eq!(facts.loops().len(), 1);
    assert_eq!(loop_.latches, [latch]);
    assert_eq!(
        loop_.entry_edges,
        [ControlEdge {
            from: entry,
            to: header
        }]
    );
    assert_eq!(loop_.exit_edges.len(), 1);
    assert_eq!(loop_.exit_edges[0].to, exit);
    assert_eq!((loop_.parent, loop_.depth), (None, 1));
    assert!(!loop_.contains_irreducible);
    assert_eq!(facts.loop_for(latch).unwrap().header, header);
    assert!(facts.loop_for(exit).is_none());
    assert!(!loop_.contains(phi), "pure Phi is not a control vertex");
    for limit in [0, 1, 7, 31] {
        assert_eq!(
            Interpreter::new(&f.graph)
                .run_values(&[Value::Scalar(limit)], &mut |name: &str, _: &[Value]| Err(
                    InterpretError::UnknownCallee(name.into())
                ))
                .unwrap(),
            SemanticOutcome::Return(Value::Scalar(limit))
        );
    }
}

#[test]
fn two_latches_share_one_header_and_one_natural_loop() {
    let mut f = Fixture::new();
    let entry = f.jump(f.entry);
    let header = f.region(&[entry]);
    let (body, exit) = f.branch(header, f.predicate);
    let (left, right) = f.branch(body, f.predicate);
    let a = f.jump(left);
    let b = f.jump(right);
    f.graph.set_inputs(header, &[entry, a, b]);
    f.ret(exit, f.input);
    let facts = f.facts();
    assert_eq!(facts.loops().len(), 1);
    let loop_ = facts.loop_by_header(header).unwrap();
    assert_eq!(loop_.latches, [a, b]);
    assert!(loop_.contains(left) && loop_.contains(right));
    assert_eq!(facts.stats().natural_backedges, 2);
    assert!(facts.irreducible_regions().is_empty());
}

#[test]
fn nested_loop_members_include_inner_loop_but_use_innermost_depth() {
    let mut f = Fixture::new();
    let entry = f.jump(f.entry);
    let outer = f.region(&[entry]);
    let (body, exit) = f.branch(outer, f.predicate);
    let inner_entry = f.jump(body);
    let inner = f.region(&[inner_entry]);
    let (inner_body, inner_exit) = f.branch(inner, f.predicate);
    let inner_latch = f.jump(inner_body);
    let outer_latch = f.jump(inner_exit);
    f.graph.set_inputs(inner, &[inner_entry, inner_latch]);
    f.graph.set_inputs(outer, &[entry, outer_latch]);
    f.ret(exit, f.input);
    let facts = f.facts();
    let outer_loop = facts.loop_by_header(outer).unwrap();
    let inner_loop = facts.loop_by_header(inner).unwrap();
    assert_eq!(facts.loops().len(), 2);
    assert_eq!((inner_loop.parent, inner_loop.depth), (Some(outer), 2));
    assert!(
        inner_loop
            .members
            .iter()
            .all(|&node| outer_loop.contains(node))
    );
    assert_eq!(facts.loop_for(inner_latch).unwrap().header, inner);
    assert_eq!(facts.loop_for(outer_latch).unwrap().header, outer);
    assert!(!outer_loop.contains_irreducible);
}

fn irreducible_fixture(nested: bool) -> (Fixture, NodeId, NodeId, Option<NodeId>) {
    let mut f = Fixture::new();
    let mut outer = None;
    let (entry_control, final_exit) = if nested {
        let entry = f.jump(f.entry);
        let header = f.region(&[entry]);
        outer = Some(header);
        let (body, exit) = f.branch(header, f.predicate);
        (body, Some(exit))
    } else {
        (f.entry, None)
    };
    let (left, right) = f.branch(entry_control, f.predicate);
    let left_entry = f.jump(left);
    let right_entry = f.jump(right);
    let a = f.region(&[left_entry]);
    let b = f.region(&[right_entry]);
    let (a_continue, a_exit) = f.branch(a, f.predicate);
    let (b_continue, b_exit) = f.branch(b, f.predicate);
    let to_b = f.jump(a_continue);
    let to_a = f.jump(b_continue);
    f.graph.set_inputs(a, &[left_entry, to_a]);
    f.graph.set_inputs(b, &[right_entry, to_b]);
    let a_exit = f.jump(a_exit);
    let b_exit = f.jump(b_exit);
    let join = f.region(&[a_exit, b_exit]);
    if let Some(header) = outer {
        let latch = f.jump(join);
        let entry = f.graph.input(header, 0);
        f.graph.set_inputs(header, &[entry, latch]);
        f.ret(final_exit.unwrap(), f.input);
    } else {
        f.ret(join, f.input);
    }
    (f, a, b, outer)
}

#[test]
fn multiple_entry_cycle_is_irreducible_not_a_natural_loop() {
    let (mut f, a, b, _) = irreducible_fixture(false);
    let facts = f.facts();
    assert!(facts.loops().is_empty());
    assert_eq!(facts.irreducible_regions().len(), 1);
    assert_eq!(facts.irreducible_regions()[0].entry_edges.len(), 2);
    assert!(facts.is_irreducible(a) && facts.is_irreducible(b));
}

#[test]
fn irreducible_inner_region_is_not_hidden_by_natural_outer_scc() {
    let (mut f, a, b, outer) = irreducible_fixture(true);
    let facts = f.facts();
    assert_eq!(facts.loops().len(), 1);
    let loop_ = facts.loop_by_header(outer.unwrap()).unwrap();
    assert!(loop_.contains(a) && loop_.contains(b));
    assert!(loop_.contains_irreducible);
    assert_eq!(facts.irreducible_regions().len(), 1);
    assert!(facts.is_irreducible(a) && facts.is_irreducible(b));
    assert!(!facts.is_irreducible(outer.unwrap()));
}
