//! Atomically change a Region's predecessor slots and all corresponding Phis.

use crate::{FunctionGraph, NodeId, Opcode};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CfgEditError {
    InvalidRegion(NodeId),
    WrongPredecessorMask {
        region: NodeId,
        expected: usize,
        actual: usize,
    },
    EmptyRegion(NodeId),
    MalformedPhi {
        region: NodeId,
        phi: NodeId,
    },
}

impl std::fmt::Display for CfgEditError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "cannot edit control graph: {self:?}")
    }
}

impl std::error::Error for CfgEditError {}

pub struct CfgEditor<'a> {
    graph: &'a mut FunctionGraph,
}

impl<'a> CfgEditor<'a> {
    pub fn new(graph: &'a mut FunctionGraph) -> Self {
        Self { graph }
    }

    /// Removes slots with a false mask entry, preserving the remaining order.
    /// Every Phi is validated before any mutation. An unreachable Region is
    /// retired by the caller together with its control subgraph, not emptied.
    pub fn retain_predecessors(
        &mut self,
        region: NodeId,
        keep: &[bool],
    ) -> Result<usize, CfgEditError> {
        let graph = &mut self.graph;
        if region.is_none()
            || region.index() >= graph.len()
            || graph.is_dead(region)
            || graph.op(region) != Opcode::Region
        {
            return Err(CfgEditError::InvalidRegion(region));
        }
        let count = graph.inputs(region).len();
        if keep.len() != count {
            return Err(CfgEditError::WrongPredecessorMask {
                region,
                expected: count,
                actual: keep.len(),
            });
        }
        let removed = keep.iter().filter(|retain| !**retain).count();
        if removed == 0 {
            return Ok(0);
        }
        if removed == count {
            return Err(CfgEditError::EmptyRegion(region));
        }
        let mut updates = Vec::new();
        for phi in graph.ids().filter(|node| graph.op(*node) == Opcode::Phi) {
            if graph.inputs(phi).first().copied() != Some(region) {
                continue;
            }
            if graph.inputs(phi).len() != count + 1 {
                return Err(CfgEditError::MalformedPhi { region, phi });
            }
            let mut inputs = vec![region];
            inputs.extend(
                graph.inputs(phi)[1..]
                    .iter()
                    .zip(keep)
                    .filter_map(|(&value, &retain)| retain.then_some(value)),
            );
            updates.push((phi, inputs));
        }
        let predecessors: Vec<_> = graph
            .inputs(region)
            .iter()
            .zip(keep)
            .filter_map(|(&jump, &retain)| retain.then_some(jump))
            .collect();
        for (phi, inputs) in updates {
            if inputs.len() == 2 && inputs[1] != phi {
                graph.replace(phi, inputs[1]);
            } else {
                graph.set_inputs(phi, &inputs);
            }
        }
        graph.set_inputs(region, &predecessors);
        Ok(removed)
    }
}
