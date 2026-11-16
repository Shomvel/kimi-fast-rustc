use super::{Work, expression};
use crate::analysis::{
    cfg::{ControlFlow, is_anchored_value, is_control_flow},
    dominators::Dominators,
    loops::LoopInfo,
    uses::{UseKind, Uses},
};
use crate::{FunctionGraph, NodeId, Opcode};
use kfrc_body::fx_hash::{HashMap, HashSet};

pub(super) fn live_nodes(
    graph: &FunctionGraph,
    cfg: &ControlFlow,
    nodes: &[NodeId],
    work: &mut Work,
) -> Option<Vec<bool>> {
    let mut live = vec![false; graph.len()];
    let mut pending: Vec<_> = nodes
        .iter()
        .copied()
        .filter(|&node| {
            (is_control_flow(graph, node) && cfg.contains(node))
                || (is_anchored_value(graph.op(node)) && cfg.contains(graph.control(node)))
        })
        .collect();
    while let Some(node) = pending.pop() {
        work.charge(1)?;
        let node = graph.resolve(node);
        if node.is_none() || live[node.index()] {
            continue;
        }
        live[node.index()] = true;
        pending.extend_from_slice(graph.inputs(node));
        pending.push(graph.control(node));
    }
    Some(live)
}

pub(super) struct Proof<'a> {
    graph: &'a FunctionGraph,
    pub cfg: &'a ControlFlow,
    dominators: &'a Dominators,
    loops: &'a LoopInfo,
    uses: &'a Uses,
    live: &'a [bool],
    sites: HashMap<NodeId, Vec<NodeId>>,
    available: HashMap<(NodeId, NodeId), bool>,
    pub work: &'a mut Work,
}

impl<'a> Proof<'a> {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        graph: &'a FunctionGraph,
        cfg: &'a ControlFlow,
        dominators: &'a Dominators,
        loops: &'a LoopInfo,
        uses: &'a Uses,
        live: &'a [bool],
        work: &'a mut Work,
    ) -> Self {
        Self {
            graph,
            cfg,
            dominators,
            loops,
            uses,
            live,
            sites: HashMap::default(),
            available: HashMap::default(),
            work,
        }
    }

    pub fn acyclic(&self, control: NodeId) -> bool {
        self.cfg.contains(control)
            && self.loops.loop_for(control).is_none()
            && !self.loops.is_irreducible(control)
    }

    pub fn dominates(&self, from: NodeId, to: NodeId) -> bool {
        self.dominators.dominates(self.cfg, from, to)
    }

    pub fn available(&mut self, source: NodeId, pred: NodeId) -> bool {
        self.available_at(source, pred, &mut HashSet::default(), 0)
            .unwrap_or(false)
    }

    fn available_at(
        &mut self,
        source: NodeId,
        pred: NodeId,
        active: &mut HashSet<NodeId>,
        depth: usize,
    ) -> Option<bool> {
        self.work.charge(1)?;
        let source = self.graph.resolve(source);
        if let Some(&known) = self.available.get(&(source, pred)) {
            return Some(known);
        }
        if depth == 32 {
            self.work.limited += 1;
            return None;
        }
        if !expression::scalar(self.graph, source) || !active.insert(source) {
            return Some(false);
        }
        let graph = self.graph;
        let result = match graph.op(source) {
            Opcode::Const | Opcode::Undefined => true,
            Opcode::Phi => {
                let region = graph.input(source, 0);
                self.acyclic(region) && self.dominates(region, pred)
            }
            Opcode::Projection => {
                let parent = graph.input(source, 0);
                // External snapshot/opaque results are immutable values. The
                // first scope deliberately excludes memory and call results.
                matches!(graph.op(parent), Opcode::Start | Opcode::Opaque)
                    && self.acyclic(parent)
                    && self.dominates(parent, pred)
            }
            _ if expression::eligible(graph, source) => {
                let mut all = true;
                for &input in graph.inputs(source) {
                    all &= self.available_at(input, pred, active, depth + 1)?;
                }
                all
            }
            _ => false,
        };
        active.remove(&source);
        self.available.insert((source, pred), result);
        Some(result)
    }

    pub fn needed_before(&mut self, expression: NodeId, pred: NodeId) -> Option<bool> {
        if !self.sites.contains_key(&expression) {
            let mut pending = vec![expression];
            let mut seen = HashSet::default();
            let mut sites = Vec::new();
            while let Some(node) = pending.pop() {
                self.work.charge(1)?;
                if !seen.insert(node) {
                    continue;
                }
                for usage in self.uses.of(node) {
                    self.work.charge(1)?;
                    let user = usage.user;
                    if !self.live[user.index()] {
                        continue;
                    }
                    if self.graph.op(user) == Opcode::Phi {
                        if let UseKind::Operand(slot) = usage.kind()
                            && slot > 0
                        {
                            sites.push(self.graph.input(self.graph.input(user, 0), slot - 1));
                        }
                    } else if is_anchored_value(self.graph.op(user)) {
                        sites.push(self.graph.control(user));
                    } else if is_control_flow(self.graph, user) {
                        sites.push(user);
                    } else if expression::scalar_op(self.graph.op(user))
                        && expression::scalar(self.graph, user)
                    {
                        pending.push(user);
                    }
                }
            }
            sites.sort_unstable();
            sites.dedup();
            self.sites.insert(expression, sites);
        }
        Some(
            self.sites[&expression]
                .iter()
                .any(|&site| self.acyclic(site) && self.dominates(site, pred)),
        )
    }
}
