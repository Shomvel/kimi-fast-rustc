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
        // One walk records each edge as (used, user, slot) in the order of the
        // per-node lists (users by id, operand slots, then control); a counting
        // sort by used node then places the uses.
        const CONTROL: u32 = u32::MAX;
        let mut starts = vec![0u32; graph.len() + 1];
        let mut edges: Vec<[u32; 3]> = Vec::with_capacity(graph.len() * 2);
        for user in graph.ids() {
            for (slot, &input) in graph.inputs(user).iter().enumerate() {
                let used = graph.resolve(input);
                starts[used.index() + 1] += 1;
                edges.push([used.0, user.0, slot as u32]);
            }
            let control = graph.resolve(graph.control(user));
            if !control.is_none() {
                starts[control.index() + 1] += 1;
                edges.push([control.0, user.0, CONTROL]);
            }
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
        for [used, user, slot] in edges {
            let position = &mut next[used as usize];
            entries[*position as usize] = Use {
                user: NodeId(user),
                kind: if slot == CONTROL {
                    UseKind::Control
                } else {
                    UseKind::Operand(slot as usize)
                },
            };
            *position += 1;
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
