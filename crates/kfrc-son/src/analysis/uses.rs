//! Operand and control uses resolved through the current forwarding table.
//! Unchanged graphs share one immutable index; edits discard the graph's
//! cached handle without changing snapshots already held by a transform.
use crate::graph::{FunctionGraph, NodeId};
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UseKind {
    Operand(usize),
    Control,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Use {
    pub user: NodeId,
    pub kind: UseKind,
}
/// Uses grouped by used node in one flat array (a counting sort of the
/// graph's edges), in the order of the per-node lists it replaces: users by
/// node id, then operand slots, then the control use.
#[derive(Clone, Debug)]
pub struct Uses {
    index: Arc<UseIndex>,
}

#[derive(Debug)]
struct UseIndex {
    starts: Vec<u32>,
    entries: Vec<Use>,
}
impl Uses {
    pub fn new(graph: &FunctionGraph) -> Self {
        graph.uses()
    }

    pub(crate) fn build(graph: &FunctionGraph) -> Self {
        let mut edges: Vec<(NodeId, Use)> = Vec::with_capacity(graph.len() * 2);
        for user in graph.ids() {
            for (slot, &input) in graph.inputs(user).iter().enumerate() {
                edges.push((
                    graph.resolve(input),
                    Use {
                        user,
                        kind: UseKind::Operand(slot),
                    },
                ));
            }
            let control = graph.resolve(graph.control(user));
            if !control.is_none() {
                edges.push((
                    control,
                    Use {
                        user,
                        kind: UseKind::Control,
                    },
                ));
            }
        }
        let mut starts = vec![0u32; graph.len() + 1];
        for (used, _) in &edges {
            starts[used.index() + 1] += 1;
        }
        for index in 1..starts.len() {
            starts[index] += starts[index - 1];
        }
        let mut next = starts.clone();
        let placeholder = Use {
            user: NodeId::NONE,
            kind: UseKind::Control,
        };
        let mut entries = vec![placeholder; edges.len()];
        for (used, entry) in edges {
            let slot = &mut next[used.index()];
            entries[*slot as usize] = entry;
            *slot += 1;
        }
        Self {
            index: Arc::new(UseIndex { starts, entries }),
        }
    }
    pub fn of(&self, node: NodeId) -> &[Use] {
        &self.index.entries
            [self.index.starts[node.index()] as usize..self.index.starts[node.index() + 1] as usize]
    }
    /// Whether this immutable snapshot includes the node. A rewrite may have
    /// appended records since the snapshot was built.
    pub fn contains(&self, node: NodeId) -> bool {
        !node.is_none() && node.index() + 1 < self.index.starts.len()
    }
}

#[cfg(test)]
#[path = "uses_tests.rs"]
mod tests;
