//! Operand and control uses resolved through the current forwarding table.
use crate::graph::{FunctionGraph, NodeId};

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
#[derive(Clone, Debug)]
pub struct Uses {
    entries: Vec<Vec<Use>>,
}
impl Uses {
    pub fn new(graph: &FunctionGraph) -> Self {
        let mut entries = vec![Vec::new(); graph.len()];
        for user in graph.ids() {
            for (slot, &input) in graph.inputs(user).iter().enumerate() {
                entries[graph.resolve(input).index()].push(Use {
                    user,
                    kind: UseKind::Operand(slot),
                });
            }
            let control = graph.resolve(graph.control(user));
            if !control.is_none() {
                entries[control.index()].push(Use {
                    user,
                    kind: UseKind::Control,
                });
            }
        }
        Self { entries }
    }
    pub fn of(&self, node: NodeId) -> &[Use] {
        &self.entries[node.index()]
    }
}
