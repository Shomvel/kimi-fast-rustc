//! Atomically change a Region's predecessor slots and all corresponding Phis.

use crate::{FunctionGraph, NodeId, Opcode, ValueType};

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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GroupedPredecessors {
    pub region: NodeId,
    pub jump: NodeId,
    pub phis_created: usize,
}

impl<'a> CfgEditor<'a> {
    pub fn new(graph: &'a mut FunctionGraph) -> Self {
        Self { graph }
    }

    /// Merge a proper subset of at least two incoming edges through a new
    /// Region and Jump. Every value/effect Phi is grouped in the same slot
    /// order. Validate all old Phis before publishing any new graph nodes.
    pub fn group_predecessors(
        &mut self,
        region: NodeId,
        group: &[bool],
    ) -> Result<Option<GroupedPredecessors>, CfgEditError> {
        let graph = &mut self.graph;
        if region.is_none()
            || region.index() >= graph.len()
            || graph.is_dead(region)
            || graph.op(region) != Opcode::Region
        {
            return Err(CfgEditError::InvalidRegion(region));
        }
        let predecessors = graph.inputs(region).to_vec();
        if group.len() != predecessors.len() {
            return Err(CfgEditError::WrongPredecessorMask {
                region,
                expected: predecessors.len(),
                actual: group.len(),
            });
        }
        let selected = group.iter().filter(|&&selected| selected).count();
        if selected < 2 || selected == group.len() {
            return Ok(None);
        }
        let mut phis = Vec::new();
        for phi in graph.ids().filter(|&node| graph.op(node) == Opcode::Phi) {
            if graph.inputs(phi).first().copied() != Some(region) {
                continue;
            }
            if graph.inputs(phi).len() != predecessors.len() + 1 {
                return Err(CfgEditError::MalformedPhi { region, phi });
            }
            phis.push((phi, graph.inputs(phi)[1..].to_vec()));
        }
        let edges: Vec<_> = predecessors
            .iter()
            .zip(group)
            .filter_map(|(&edge, &selected)| selected.then_some(edge))
            .collect();
        let grouped = graph.add_node(Opcode::Region, NodeId::NONE, &edges, 0, ValueType::Control);
        let jump = graph.add_node(Opcode::Jump, grouped, &[], 0, ValueType::Control);
        let mut phis_created = 0;
        for (phi, values) in phis {
            let incoming: Vec<_> = values
                .iter()
                .zip(group)
                .filter_map(|(&value, &selected)| selected.then_some(graph.resolve(value)))
                .collect();
            let first = incoming[0];
            let merged = if incoming.iter().all(|&value| value == first) {
                first
            } else {
                let mut inputs = vec![grouped];
                inputs.extend(incoming);
                phis_created += 1;
                graph.add_node(
                    Opcode::Phi,
                    NodeId::NONE,
                    &inputs,
                    0,
                    graph.value_type(phi).clone(),
                )
            };
            let mut inputs = vec![region];
            let mut inserted = false;
            for (&value, &selected) in values.iter().zip(group) {
                if !selected {
                    inputs.push(value);
                } else if !inserted {
                    inputs.push(merged);
                    inserted = true;
                }
            }
            graph.set_inputs(phi, &inputs);
        }
        let mut inputs = Vec::new();
        let mut inserted = false;
        for (edge, &selected) in predecessors.iter().zip(group) {
            if !selected {
                inputs.push(*edge);
            } else if !inserted {
                inputs.push(jump);
                inserted = true;
            }
        }
        graph.set_inputs(region, &inputs);
        Ok(Some(GroupedPredecessors {
            region: grouped,
            jump,
            phis_created,
        }))
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
