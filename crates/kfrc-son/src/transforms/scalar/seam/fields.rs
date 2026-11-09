//! A lookup index built alongside the seam's existing use index. Only visited
//! seeds and their replacement users are rewritten; this is not a GVN sweep.

mod invariant;

use std::collections::HashMap;

use crate::{FunctionGraph, NodeId, Opcode, ValueType};

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct Field {
    source: NodeId,
    index: u32,
    ty: ValueType,
}

impl Field {
    fn of(graph: &FunctionGraph, node: NodeId) -> Option<Self> {
        if graph.op(node) != Opcode::Field || !graph.control(node).is_none() {
            return None;
        }
        let [source] = graph.inputs(node) else {
            return None;
        };
        Some(Self {
            source: graph.resolve(*source),
            index: graph.payload(node),
            ty: graph.value_type(node).clone(),
        })
    }
}

#[derive(Default)]
pub(super) struct Fields(HashMap<Field, NodeId>);

impl Fields {
    pub(super) fn record(&mut self, graph: &FunctionGraph, node: NodeId) {
        if let Some(key) = Field::of(graph, node) {
            self.0.entry(key).or_insert(node);
        }
    }

    pub(super) fn replacement(&mut self, graph: &FunctionGraph, node: NodeId) -> Option<NodeId> {
        let key = Field::of(graph, node)?;
        if let Some(value) = invariant::find(graph, key.source, key.index, &key.ty) {
            return Some(value);
        }
        if let Some(&candidate) = self.0.get(&key) {
            let candidate = graph.resolve(candidate);
            // Forwarding may have changed this representative or its source.
            // Validate the current expression instead of trusting an old key.
            if !graph.is_dead(candidate) && Field::of(graph, candidate).as_ref() == Some(&key) {
                return (candidate != node).then_some(candidate);
            }
        }
        self.0.insert(key, node);
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kfrc_body::{FunctionAbi, IntegerWidth, PassMode, RecordedTypes, TypeShape};

    #[test]
    fn stale_representative_is_revalidated_after_its_source_changes() {
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
            "field_index",
            FunctionAbi {
                args: vec![],
                ret: PassMode::Ignore,
                c_variadic: false,
            },
            types,
        );
        let sources: [NodeId; 2] = std::array::from_fn(|_| {
            graph.add_node(
                Opcode::Undefined,
                NodeId::NONE,
                &[],
                0,
                ValueType::Rust(pair),
            )
        });
        let mut field = |source| {
            graph.add_node(
                Opcode::Field,
                NodeId::NONE,
                &[source],
                0,
                ValueType::Rust(word),
            )
        };
        let original = field(sources[0]);
        let same = field(sources[0]);
        let other = field(sources[1]);
        let mut fields = Fields::default();
        fields.record(&graph, original);
        assert_eq!(fields.replacement(&graph, same), Some(original));
        assert_eq!(fields.replacement(&graph, other), None);
        graph.set_input(original, 0, sources[1]);
        assert_eq!(fields.replacement(&graph, same), None);
        assert_ne!(graph.resolve(same), graph.resolve(original));
    }
}
