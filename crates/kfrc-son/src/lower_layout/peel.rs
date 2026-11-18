//! Peel one iteration when an entry constant can remove checked arithmetic.
//! The original loop keeps its backedge. Exit merges repair both values and
//! effects, so no load, call or trapping operation moves onto another path.

use super::*;
use crate::analysis::{
    cfg::ControlFlow,
    dominators::Dominators,
    uses::{UseKind, Uses},
};

#[derive(Default)]
pub(super) struct Stats {
    pub loops: usize,
    pub nodes_added: usize,
}

struct Exit {
    node: NodeId,
    uses: Vec<(NodeId, UseKind)>,
}

#[derive(Clone, Copy)]
enum ValuePlace {
    Original,
    Exit(usize),
    Merge(usize),
    Expression(usize),
}

struct RepairPhi {
    value: NodeId,
    region: NodeId,
    inputs: Vec<ValuePlace>,
}

struct RepairExpression {
    value: NodeId,
    inputs: Vec<(NodeId, ValuePlace)>,
}

struct Repair<'a> {
    graph: &'a FunctionGraph,
    cfg: &'a ControlFlow,
    dominators: &'a Dominators,
    header: NodeId,
    dependent: &'a HashSet<NodeId>,
    bases: &'a HashSet<NodeId>,
    exits: &'a [Exit],
    memo: HashMap<(NodeId, NodeId), ValuePlace>,
    phis: Vec<RepairPhi>,
    expressions: Vec<RepairExpression>,
    fuel: usize,
}

impl Repair<'_> {
    fn at(&mut self, value: NodeId, control: NodeId, depth: usize) -> Option<ValuePlace> {
        if !self.dependent.contains(&value) {
            return Some(ValuePlace::Original);
        }
        if let Some(&known) = self.memo.get(&(value, control)) {
            return Some(known);
        }
        self.fuel = self.fuel.checked_sub(1)?;
        if depth == 64 || control.is_none() {
            return None;
        }
        if !self.bases.contains(&value) {
            let mut inputs = Vec::new();
            for &input in self.graph.inputs(value) {
                inputs.push((input, self.at(input, control, depth + 1)?));
            }
            let place = ValuePlace::Expression(self.expressions.len());
            self.expressions.push(RepairExpression { value, inputs });
            self.memo.insert((value, control), place);
            return Some(place);
        }
        if let Some(exit) = self.exits.iter().position(|e| e.node == control) {
            let definition = match self.graph.op(value) {
                Opcode::Phi | Opcode::Projection => self.graph.input(value, 0),
                _ => self.graph.control(value),
            };
            return self
                .dominators
                .dominates(self.cfg, definition, control)
                .then_some(ValuePlace::Exit(exit));
        }
        if control == self.header || !self.dominators.dominates(self.cfg, self.header, control) {
            return None;
        }
        let position = *self.cfg.position.get(&control)?;
        let predecessors: Vec<_> = self.cfg.predecessors[position]
            .iter()
            .map(|&p| self.cfg.order[p])
            .collect();
        let place = if predecessors.len() == 1 {
            self.at(value, predecessors[0], depth + 1)?
        } else {
            if self.graph.op(control) != Opcode::Region || predecessors.is_empty() {
                return None;
            }
            let index = self.phis.len();
            let place = ValuePlace::Merge(index);
            self.phis.push(RepairPhi {
                value,
                region: control,
                inputs: Vec::new(),
            });
            self.memo.insert((value, control), place);
            for &edge in self.graph.inputs(control) {
                let incoming = self.at(value, edge, depth + 1)?;
                self.phis[index].inputs.push(incoming);
            }
            place
        };
        self.memo.insert((value, control), place);
        Some(place)
    }
}

struct Plan {
    header: NodeId,
    entry_slot: usize,
    latch: NodeId,
    phis: Vec<NodeId>,
    copies: Vec<NodeId>,
    exits: Vec<Exit>,
    rewrites: Vec<(NodeId, usize, NodeId, ValuePlace)>,
    repairs: Vec<RepairPhi>,
    expressions: Vec<RepairExpression>,
}

fn pure(graph: &FunctionGraph, node: NodeId) -> bool {
    graph.control(node).is_none()
        && matches!(
            graph.op(node),
            Opcode::Add
                | Opcode::Sub
                | Opcode::Mul
                | Opcode::MulOverflow
                | Opcode::BitAnd
                | Opcode::BitOr
                | Opcode::BitXor
                | Opcode::BitNot
                | Opcode::Negate
                | Opcode::Compare
                | Opcode::IntCast
                | Opcode::ShiftLeft
                | Opcode::ShiftRightLogical
                | Opcode::ShiftRightArithmetic
                | Opcode::BitCount
                | Opcode::ByteSwap
                | Opcode::BitReverse
                | Opcode::FunnelShift
                | Opcode::SaturatingAdd
                | Opcode::SaturatingSub
                | Opcode::PointerCast
                | Opcode::PtrOffset
        )
}

impl Plan {
    fn new(
        graph: &FunctionGraph,
        cfg: &ControlFlow,
        dominators: &Dominators,
        uses: &Uses,
        loop_: &crate::analysis::loops::NaturalLoop,
    ) -> Option<Self> {
        let header = loop_.header;
        if loop_.contains_irreducible
            || loop_.parent.is_some()
            || loop_.latches.len() != 1
            || loop_.entry_edges.len() != 1
            || graph.op(header) != Opcode::Region
            || graph.inputs(header).len() != 2
            || loop_.members.len() > 64
            || loop_.exit_edges.len() > 8
        {
            return None;
        }
        let latch = loop_.latches[0];
        if graph.op(latch) != Opcode::Jump || loop_.entry_edges[0].to != header {
            return None;
        }
        let entry_slot = graph.inputs(header).iter().position(|&n| n != latch)?;
        let phis: Vec<_> = uses
            .of(header)
            .iter()
            .filter_map(|u| {
                (graph.op(u.user) == Opcode::Phi && u.kind() == UseKind::Operand(0))
                    .then_some(u.user)
            })
            .collect();
        // Profitability is structural: an initially-zero recurrence feeds a
        // multiply-overflow predicate. Ordinary iteration counting is left to
        // the existing complete unroller.
        let useful = phis.iter().any(|&phi| {
            let entry = graph.input(phi, entry_slot + 1);
            graph.op(entry) == Opcode::Const
                && graph.constant_value(entry) == 0
                && uses
                    .of(phi)
                    .iter()
                    .any(|u| graph.op(u.user) == Opcode::MulOverflow)
        });
        if !useful {
            return None;
        }
        let mut exits = Vec::new();
        for edge in &loop_.exit_edges {
            let node = if graph.op(edge.to) == Opcode::Projection
                && graph.input(edge.to, 0) == edge.from
            {
                edge.to
            } else if graph.op(edge.from) == Opcode::Jump && graph.op(edge.to) == Opcode::Region {
                edge.from
            } else {
                return None;
            };
            if exits.iter().any(|e: &Exit| e.node == node) {
                continue;
            }
            let external = uses
                .of(node)
                .iter()
                .filter(|u| !loop_.contains(u.user))
                .map(|u| (u.user, u.kind()))
                .collect();
            exits.push(Exit {
                node,
                uses: external,
            });
        }
        if exits.is_empty() {
            return None;
        }
        let mut copies: HashSet<_> = loop_
            .members
            .iter()
            .copied()
            .filter(|&n| n != header)
            .collect();
        copies.extend(exits.iter().map(|e| e.node));
        let mut bases: HashSet<_> = phis.iter().copied().collect();
        for node in graph.ids() {
            if graph.op(node) == Opcode::Phi && loop_.contains(graph.input(node, 0)) {
                bases.insert(node);
                if graph.input(node, 0) != header {
                    copies.insert(node);
                }
            } else if graph.op(node) == Opcode::Projection
                && loop_.contains(graph.input(node, 0))
                && graph.value_type(node) != &ValueType::Control
            {
                bases.insert(node);
                copies.insert(node);
            } else if !graph.control(node).is_none()
                && loop_.contains(graph.control(node))
                && !cfg.contains(node)
            {
                // Anchored values require their complete instruction semantics;
                // keep this first implementation to the scalar control spine.
                return None;
            }
        }
        let mut dependent = bases.clone();
        let mut pending: Vec<_> = bases.iter().copied().collect();
        while let Some(node) = pending.pop() {
            for u in uses.of(node) {
                if pure(graph, u.user) && dependent.insert(u.user) {
                    if dependent.len() > 192 {
                        return None;
                    }
                    pending.push(u.user);
                }
            }
        }
        // Clone only the dependent cone demanded by executable loop nodes.
        let mut pending: Vec<_> = copies.iter().copied().collect();
        pending.extend(phis.iter().map(|&phi| graph.input(phi, 2 - entry_slot)));
        let mut seen = HashSet::default();
        while let Some(node) = pending.pop() {
            if !seen.insert(node) || phis.contains(&node) {
                continue;
            }
            if dependent.contains(&node) {
                copies.insert(node);
            }
            if copies.contains(&node) {
                pending.extend(graph.inputs(node).iter().copied());
            }
        }
        if copies.len() > 128 {
            return None;
        }
        let mut repair = Repair {
            graph,
            cfg,
            dominators,
            header,
            dependent: &dependent,
            bases: &bases,
            exits: &exits,
            memo: HashMap::default(),
            phis: Vec::new(),
            expressions: Vec::new(),
            fuel: 2048,
        };
        let mut ordered: Vec<_> = dependent.iter().copied().collect();
        ordered.sort_unstable();
        let mut rewrites = Vec::new();
        for value in ordered {
            for u in uses.of(value) {
                if copies.contains(&u.user) || phis.contains(&u.user) || dependent.contains(&u.user)
                {
                    continue;
                }
                let UseKind::Operand(slot) = u.kind() else {
                    return None;
                };
                let control = if graph.op(u.user) == Opcode::Phi {
                    if slot == 0 {
                        return None;
                    }
                    graph.input(graph.input(u.user, 0), slot - 1)
                } else {
                    graph.control(u.user)
                };
                let place = repair.at(value, control, 0)?;
                rewrites.push((u.user, slot, value, place));
            }
        }
        let repairs = repair.phis;
        let expressions = repair.expressions;
        if copies.len() + repairs.len() + expressions.len() + exits.len() * (bases.len() + 3) > 256
        {
            return None;
        }
        let mut copies: Vec<_> = copies.into_iter().collect();
        copies.sort_unstable();
        Some(Self {
            header,
            entry_slot,
            latch,
            phis,
            copies,
            exits,
            rewrites,
            repairs,
            expressions,
        })
    }

    fn apply(self, graph: &mut FunctionGraph) {
        let entry = graph.input(self.header, self.entry_slot);
        let first = graph.add_node(
            Opcode::Region,
            NodeId::NONE,
            &[entry],
            0,
            ValueType::Control,
        );
        let mut map = HashMap::from_iter([(self.header, first)]);
        for &phi in &self.phis {
            map.insert(phi, graph.input(phi, self.entry_slot + 1));
        }
        for &node in &self.copies {
            let copy = graph.add_node(
                graph.op(node),
                NodeId::NONE,
                &[],
                graph.payload(node),
                graph.value_type(node).clone(),
            );
            map.insert(node, copy);
        }
        let mapped = |n: NodeId| map.get(&n).copied().unwrap_or(n);
        for &node in &self.copies {
            let inputs: Vec<_> = graph.inputs(node).iter().copied().map(mapped).collect();
            graph.set_control(map[&node], mapped(graph.control(node)));
            graph.set_inputs(map[&node], &inputs);
        }
        graph.set_input(self.header, self.entry_slot, map[&self.latch]);
        for &phi in &self.phis {
            let next = mapped(graph.input(phi, 2 - self.entry_slot));
            graph.set_input(phi, self.entry_slot + 1, next);
        }
        let mut regions = Vec::new();
        for exit in &self.exits {
            let edges: Vec<_> = [exit.node, map[&exit.node]]
                .into_iter()
                .map(|node| {
                    if graph.op(node) == Opcode::Jump {
                        node
                    } else {
                        graph.add_node(Opcode::Jump, node, &[], 0, ValueType::Control)
                    }
                })
                .collect();
            let region =
                graph.add_node(Opcode::Region, NodeId::NONE, &edges, 0, ValueType::Control);
            regions.push(region);
            let control = if graph.op(exit.node) == Opcode::Jump {
                graph.add_node(Opcode::Jump, region, &[], 0, ValueType::Control)
            } else {
                region
            };
            for &(user, kind) in &exit.uses {
                match kind {
                    UseKind::Control => graph.set_control(user, control),
                    UseKind::Operand(slot) => graph.set_input(user, slot, control),
                }
            }
        }
        let merged: Vec<_> = self
            .repairs
            .iter()
            .map(|r| {
                graph.add_node(
                    Opcode::Phi,
                    NodeId::NONE,
                    &[r.region],
                    0,
                    graph.value_type(r.value).clone(),
                )
            })
            .collect();
        let mut values = Materialize {
            plan: &self,
            map: &map,
            regions: &regions,
            merged: &merged,
            exits: HashMap::default(),
            expressions: vec![NodeId::NONE; self.expressions.len()],
        };
        for (repair, &phi) in self.repairs.iter().zip(&merged) {
            let mut inputs = vec![repair.region];
            inputs.extend(
                repair
                    .inputs
                    .iter()
                    .map(|&p| values.at(graph, repair.value, p)),
            );
            graph.set_inputs(phi, &inputs);
        }
        for &(user, slot, value, place) in &self.rewrites {
            let value = values.at(graph, value, place);
            graph.set_input(user, slot, value);
        }
        graph.resolve_edges();
    }
}

struct Materialize<'a> {
    plan: &'a Plan,
    map: &'a HashMap<NodeId, NodeId>,
    regions: &'a [NodeId],
    merged: &'a [NodeId],
    exits: HashMap<(NodeId, usize), NodeId>,
    expressions: Vec<NodeId>,
}

impl Materialize<'_> {
    fn at(&mut self, graph: &mut FunctionGraph, value: NodeId, place: ValuePlace) -> NodeId {
        match place {
            ValuePlace::Original => value,
            ValuePlace::Merge(i) => self.merged[i],
            ValuePlace::Exit(i) => *self.exits.entry((value, i)).or_insert_with(|| {
                graph.add_node(
                    Opcode::Phi,
                    NodeId::NONE,
                    &[
                        self.regions[i],
                        value,
                        self.map.get(&value).copied().unwrap_or(value),
                    ],
                    0,
                    graph.value_type(value).clone(),
                )
            }),
            ValuePlace::Expression(i) => {
                if !self.expressions[i].is_none() {
                    return self.expressions[i];
                }
                let original = self.plan.expressions[i].value;
                let mut inputs = Vec::new();
                for slot in 0..self.plan.expressions[i].inputs.len() {
                    let (input, place) = self.plan.expressions[i].inputs[slot];
                    inputs.push(self.at(graph, input, place));
                }
                let node = graph.add_node(
                    graph.op(original),
                    NodeId::NONE,
                    &inputs,
                    graph.payload(original),
                    graph.value_type(original).clone(),
                );
                self.expressions[i] = node;
                node
            }
        }
    }
}

pub(super) fn run(graph: &mut FunctionGraph) -> Stats {
    let mut stats = Stats::default();
    if graph.level != GraphLevel::Layout
        || graph.len() > 8192
        || !graph.ids().any(|n| graph.op(n) == Opcode::MulOverflow)
    {
        return stats;
    }
    graph.resolve_edges();
    let cfg = ControlFlow::new(graph);
    let loops = cfg.loop_info();
    let dominators = Dominators::new(&cfg);
    let uses = Uses::new(graph);
    let mut plans = Vec::new();
    for loop_ in loops.loops() {
        if !plans.is_empty() {
            break;
        }
        if loops
            .loops()
            .iter()
            .any(|inner| inner.parent == Some(loop_.header))
        {
            continue;
        }
        if let Some(plan) = Plan::new(graph, &cfg, &dominators, &uses, loop_) {
            plans.push(plan);
        }
    }
    let before = graph.len();
    for plan in plans {
        plan.apply(graph);
        stats.loops += 1;
    }
    stats.nodes_added = graph.len() - before;
    stats
}

#[cfg(test)]
#[path = "peel_tests.rs"]
mod tests;
