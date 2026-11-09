use super::*;
use kfrc_body::{FunctionAbi, IntegerWidth, PassMode, RecordedTypes, TyId, TypeShape};

struct Fixture {
    graph: FunctionGraph,
    word: TyId,
    pair: TyId,
    region: NodeId,
}
impl Fixture {
    fn new() -> Self {
        let mut types = RecordedTypes::new();
        let word = types.intern(
            TypeShape::Int {
                width: IntegerWidth::W64,
                signed: false,
            },
            None,
        );
        let pair = types.intern(TypeShape::Tuple(vec![word, word]), None);
        let mut graph = FunctionGraph::new(
            "field_invariant",
            FunctionAbi {
                args: vec![],
                ret: PassMode::Ignore,
                c_variadic: false,
            },
            types,
        );
        let region = graph.add_node(Opcode::Region, NodeId::NONE, &[], 0, ValueType::Control);
        Self {
            graph,
            word,
            pair,
            region,
        }
    }
    fn aggregate(&mut self, value: NodeId) -> NodeId {
        self.graph.add_node(
            Opcode::Aggregate,
            NodeId::NONE,
            &[value, value],
            0,
            ValueType::Rust(self.pair),
        )
    }
    fn phi(&mut self, values: &[NodeId]) -> NodeId {
        let mut inputs = vec![self.region];
        inputs.extend_from_slice(values);
        self.graph.add_node(
            Opcode::Phi,
            NodeId::NONE,
            &inputs,
            0,
            ValueType::Rust(self.pair),
        )
    }
    fn query(&self, source: NodeId, field: u32) -> Option<NodeId> {
        find(&self.graph, source, field, &ValueType::Rust(self.word))
    }
}

#[test]
fn changed_fields_and_ungrounded_subcycles_reject_invariance() {
    let mut f = Fixture::new();
    let zero = f.graph.add_constant(0, f.word);
    let one = f.graph.add_constant(1, f.word);
    let initial = f.aggregate(zero);
    let state = f.phi(&[initial]);
    let update = f.graph.add_node(
        Opcode::InsertField,
        NodeId::NONE,
        &[state, one],
        0,
        ValueType::Rust(f.pair),
    );
    f.graph.set_inputs(state, &[f.region, initial, update]);
    assert_eq!(f.query(state, 1), Some(zero));
    assert_eq!(f.query(state, 0), None);
    let ungrounded = f.phi(&[]);
    f.graph.set_inputs(ungrounded, &[f.region, ungrounded]);
    assert_eq!(f.query(ungrounded, 1), None);
    let mixed = f.phi(&[initial, ungrounded]);
    assert_eq!(f.query(mixed, 1), None);
    let other = f.aggregate(one);
    let nested = f.phi(&[state, other]);
    assert_eq!(f.query(nested, 1), None);
}

#[test]
fn loop_local_load_call_or_phi_is_not_an_entry_anchor() {
    let mut f = Fixture::new();
    for op in [Opcode::LoadValue, Opcode::Call] {
        let origin = f.graph.add_node(
            op,
            NodeId::NONE,
            &[],
            0,
            ValueType::Tuple(vec![
                ValueType::Control,
                ValueType::Effect,
                ValueType::Rust(f.word),
            ]),
        );
        let value = f.graph.add_node(
            Opcode::Projection,
            NodeId::NONE,
            &[origin],
            2,
            ValueType::Rust(f.word),
        );
        let initial = f.aggregate(value);
        let state = f.phi(&[initial]);
        f.graph.set_inputs(state, &[f.region, initial, state]);
        assert_eq!(f.query(state, 1), None);
    }
    let zero = f.graph.add_constant(0, f.word);
    let outer = f.graph.add_node(
        Opcode::Phi,
        NodeId::NONE,
        &[f.region, zero],
        0,
        ValueType::Rust(f.word),
    );
    let initial = f.aggregate(outer);
    let state = f.phi(&[initial]);
    f.graph.set_inputs(state, &[f.region, initial, state]);
    assert_eq!(f.query(state, 1), None);
}
