//! Repair values which escape both sides of a threaded branch. The plan is
//! built on the original CFG; only pure translations and merge Phis are added.
use super::thread_variants::cloneable;
use crate::analysis::{cfg::ControlFlow, dominators::Dominators};
use crate::{FunctionGraph, NodeId, Opcode};
use kfrc_body::fx_hash::{HashMap, HashSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Placement {
    Original,
    Side(usize),
    Merge(usize),
    Expression(usize),
    Unreachable,
}

pub(super) struct Merge {
    pub value: NodeId,
    pub region: NodeId,
    pub inputs: Vec<Placement>,
}

pub(super) struct Expression {
    pub value: NodeId,
    pub inputs: Vec<(NodeId, Placement)>,
}

pub(super) struct Planner<'a> {
    graph: &'a FunctionGraph,
    cfg: &'a ControlFlow,
    dominators: &'a Dominators,
    root: NodeId,
    leaves: &'a [NodeId],
    live_sides: &'a [bool],
    phis: &'a [NodeId],
    dependent: &'a HashSet<NodeId>,
    witnesses: &'a HashSet<NodeId>,
    placements: HashMap<(NodeId, NodeId), Placement>,
    pub merges: Vec<Merge>,
    pub expressions: Vec<Expression>,
    remaining: usize,
    pub visits: usize,
}

impl<'a> Planner<'a> {
    pub fn new(
        graph: &'a FunctionGraph,
        cfg: &'a ControlFlow,
        dominators: &'a Dominators,
        root: NodeId,
        leaves: &'a [NodeId],
        live_sides: &'a [bool],
        phis: &'a [NodeId],
        dependent: &'a HashSet<NodeId>,
        witnesses: &'a HashSet<NodeId>,
    ) -> Self {
        Self {
            graph,
            cfg,
            dominators,
            root,
            leaves,
            live_sides,
            phis,
            dependent,
            witnesses,
            placements: HashMap::default(),
            merges: Vec::new(),
            expressions: Vec::new(),
            remaining: 512,
            visits: 0,
        }
    }

    fn charge(&mut self) -> Option<()> {
        self.remaining = self.remaining.checked_sub(1)?;
        self.visits += 1;
        Some(())
    }

    pub fn at(&mut self, value: NodeId, control: NodeId) -> Option<Placement> {
        self.place(value, control, 0)
    }

    fn place(&mut self, value: NodeId, control: NodeId, depth: usize) -> Option<Placement> {
        if !self.dependent.contains(&value) {
            return Some(Placement::Original);
        }
        if let Some(&placement) = self.placements.get(&(value, control)) {
            return Some(placement);
        }
        self.charge()?;
        if depth == 64 {
            return None;
        }
        if let Some(side) = self
            .leaves
            .iter()
            .position(|&leaf| self.dominators.dominates(self.cfg, leaf, control))
        {
            return Some(if !self.live_sides[side] {
                Placement::Unreachable
            } else {
                Placement::Side(side)
            });
        }
        if control == self.root || !self.dominators.dominates(self.cfg, self.root, control) {
            return None;
        }
        if !self.phis.contains(&value) {
            if !cloneable(self.graph, value) {
                return None;
            }
            // Repair the defining Phis, then rebuild the expression at this
            // use. A Phi of each derived comparison would carry redundant
            // state across joins and pull later call results onto earlier
            // edges. Nontranslated inputs keep their original definitions.
            let mut operands = self.graph.inputs(value).to_vec();
            if self.graph.op(value) == Opcode::VariantField
                && operands
                    .get(1)
                    .is_some_and(|proof| self.witnesses.contains(proof))
            {
                operands.truncate(1);
            }
            let mut inputs = Vec::with_capacity(operands.len());
            for input in operands {
                inputs.push((input, self.place(input, control, depth + 1)?));
            }
            let placement = Placement::Expression(self.expressions.len());
            self.expressions.push(Expression { value, inputs });
            self.placements.insert((value, control), placement);
            return Some(placement);
        }
        let position = *self.cfg.position.get(&control)?;
        let predecessors: Vec<_> = self.cfg.predecessors[position]
            .iter()
            .map(|&pred| self.cfg.order[pred])
            .collect();
        let placement = if predecessors.len() == 1 {
            self.place(value, predecessors[0], depth + 1)?
        } else {
            if self.graph.op(control) != Opcode::Region || predecessors.is_empty() {
                return None;
            }
            // Reserve before walking backedges. A loop may carry the repaired
            // value, but never needs a copy of any executable operation.
            let index = self.merges.len();
            let placement = Placement::Merge(index);
            self.merges.push(Merge {
                value,
                region: control,
                inputs: Vec::new(),
            });
            self.placements.insert((value, control), placement);
            let inputs = self.graph.inputs(control).to_vec();
            for input in inputs {
                let incoming = self.place(value, input, depth + 1)?;
                self.merges[index].inputs.push(incoming);
            }
            placement
        };
        self.placements.insert((value, control), placement);
        Some(placement)
    }
}
