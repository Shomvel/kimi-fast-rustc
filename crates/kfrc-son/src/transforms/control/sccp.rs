//! Sparse conditional constant propagation over values and executable controls.
//!
//! Unknown is analysis bottom, never runtime undef. Poison and unsupported
//! values are Overdefined. Publish constants only after monotone convergence;
//! CFG deletion remains the responsibility of simplify and CfgEditor.

use kfrc_body::{TypeOracle, TypeShape};

use crate::analysis::cfg::is_control_flow;
use crate::transforms::scalar::instcombine;
use crate::{FunctionGraph, NodeId, Opcode};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Value {
    Unknown,
    Constant(u64),
    Overdefined,
}

impl Value {
    fn join(self, other: Self) -> Self {
        match (self, other) {
            (Self::Unknown, value) | (value, Self::Unknown) => value,
            (Self::Constant(a), Self::Constant(b)) if a == b => self,
            _ => Self::Overdefined,
        }
    }
}

#[derive(Default)]
pub(crate) struct SccpStats {
    pub constants: usize,
    pub visits: usize,
    pub budget_bailouts: usize,
    pub dirty: Vec<NodeId>,
}

fn scalar(graph: &FunctionGraph, node: NodeId) -> bool {
    graph
        .rust_type(node)
        .is_some_and(|ty| match graph.oracle.shape(ty) {
            TypeShape::Bool => true,
            TypeShape::Int { width, .. } => width.bits(64) <= 64,
            _ => false,
        })
}

pub(crate) fn run(graph: &mut FunctionGraph) -> SccpStats {
    // InstCombine already handles straight-line literal expressions. Only pay
    // for joint control/value propagation when a scalar merge can benefit.
    if !graph
        .ids()
        .any(|n| graph.op(n) == Opcode::Phi && scalar(graph, n))
    {
        return SccpStats::default();
    }
    graph.resolve_edges();
    let mut solver = Solver::new(graph);
    let mut stats = SccpStats::default();
    if !solver.solve() {
        stats.visits = solver.visits;
        stats.budget_bailouts = 1;
        return stats;
    }
    // An unresolved cycle is not a license to ignore a possible path or poison
    // input. Widen all remaining bottoms, then propagate to convergence again.
    for node in graph.ids() {
        if solver.values[node.index()] == Value::Unknown {
            solver.values[node.index()] = Value::Overdefined;
            solver.queue_users(node);
        }
    }
    if !solver.solve() {
        stats.visits = solver.visits;
        stats.budget_bailouts = 1;
        return stats;
    }
    stats.visits = solver.visits;
    let replacements: Vec<_> = graph
        .ids()
        .filter_map(|node| {
            let Value::Constant(bits) = solver.values[node.index()] else {
                return None;
            };
            (graph.op(node) != Opcode::Const && graph.control(node).is_none())
                .then(|| (node, bits, graph.rust_type(node).unwrap()))
        })
        .collect();
    for &(node, _, _) in &replacements {
        stats.dirty.extend_from_slice(&solver.users[node.index()]);
    }
    drop(solver);
    for (node, bits, ty) in replacements {
        let replacement = graph.add_constant(u128::from(bits), ty);
        graph.replace(node, replacement);
        stats.constants += 1;
    }
    stats
}

struct Solver<'a> {
    graph: &'a FunctionGraph,
    values: Vec<Value>,
    executable: Vec<bool>,
    users: Vec<Vec<NodeId>>,
    pending: Vec<NodeId>,
    queued: Vec<bool>,
    remaining: usize,
    visits: usize,
}

impl<'a> Solver<'a> {
    fn new(graph: &'a FunctionGraph) -> Self {
        let mut users = vec![Vec::new(); graph.len()];
        let mut values = vec![Value::Overdefined; graph.len()];
        let mut queued = vec![false; graph.len()];
        let mut pending = Vec::new();
        let mut edges = 0usize;
        for node in graph.ids() {
            for &input in graph
                .inputs(node)
                .iter()
                .chain(std::iter::once(&graph.control(node)))
            {
                if !input.is_none() {
                    users[input.index()].push(node);
                    edges += 1;
                }
            }
            if scalar(graph, node) && graph.control(node).is_none() {
                values[node.index()] = match graph.op(node) {
                    Opcode::Const => Value::Constant(instcombine::constant(graph, node).unwrap()),
                    Opcode::Phi => Value::Unknown,
                    op if instcombine::supports_evaluation(op) => Value::Unknown,
                    _ => Value::Overdefined,
                };
            }
            queued[node.index()] = true;
            pending.push(node);
        }
        pending.reverse();
        Self {
            graph,
            values,
            users,
            pending,
            queued,
            executable: vec![false; graph.len()],
            remaining: graph
                .len()
                .saturating_mul(32)
                .saturating_add(edges.saturating_mul(8)),
            visits: 0,
        }
    }

    fn queue_users(&mut self, node: NodeId) {
        for &user in &self.users[node.index()] {
            if !self.queued[user.index()] {
                self.queued[user.index()] = true;
                self.pending.push(user);
            }
        }
    }

    fn active(&self, node: NodeId) -> bool {
        !node.is_none() && self.executable[node.index()]
    }

    fn control_active(&self, node: NodeId) -> bool {
        let g = self.graph;
        match g.op(node) {
            Opcode::Start => node == g.start,
            Opcode::Region => g.inputs(node).iter().any(|&p| self.active(p)),
            Opcode::Projection => {
                let source = g.input(node, 0);
                if !self.active(source) {
                    return false;
                }
                if g.op(source) == Opcode::If {
                    match self.values[g.input(source, 0).index()] {
                        Value::Unknown => false,
                        Value::Constant(0) => g.payload(node) == 1,
                        Value::Constant(1) => g.payload(node) == 0,
                        _ => true,
                    }
                } else {
                    // Includes calls/loads and VariantSwitch: conservatively
                    // enable every successor; no enum or memory lattice yet.
                    true
                }
            }
            _ => self.active(g.control(node)),
        }
    }

    fn value(&self, node: NodeId) -> Value {
        let g = self.graph;
        if g.op(node) == Opcode::Phi {
            let region = g.input(node, 0);
            return g
                .inputs(region)
                .iter()
                .zip(&g.inputs(node)[1..])
                .filter(|(predecessor, _)| self.active(**predecessor))
                .fold(Value::Unknown, |value, (_, &input)| {
                    value.join(self.values[input.index()])
                });
        }
        if g.inputs(node)
            .iter()
            .any(|n| self.values[n.index()] == Value::Overdefined)
        {
            return Value::Overdefined;
        }
        if g.inputs(node)
            .iter()
            .any(|n| self.values[n.index()] == Value::Unknown)
        {
            return Value::Unknown;
        }
        instcombine::evaluate(g, node, |input| match self.values[input.index()] {
            Value::Constant(bits) => Some(bits),
            _ => None,
        })
        .map_or(Value::Overdefined, Value::Constant)
    }

    fn solve(&mut self) -> bool {
        while let Some(node) = self.pending.pop() {
            self.queued[node.index()] = false;
            let cost = 1 + self.graph.inputs(node).len() + self.users[node.index()].len();
            let Some(remaining) = self.remaining.checked_sub(cost) else {
                return false;
            };
            self.remaining = remaining;
            self.visits += 1;
            if is_control_flow(self.graph, node) {
                let active = self.control_active(node);
                let fresh = active && !self.executable[node.index()];
                if fresh {
                    self.executable[node.index()] = true;
                }
                // If changes its feasible edges when its value changes. A
                // Region must revisit Phis when another predecessor activates,
                // even when the Region itself was already executable.
                if fresh || (active && matches!(self.graph.op(node), Opcode::If | Opcode::Region)) {
                    if !fresh && self.graph.op(node) == Opcode::Region {
                        for &user in &self.users[node.index()] {
                            if self.graph.op(user) == Opcode::Phi && !self.queued[user.index()] {
                                self.queued[user.index()] = true;
                                self.pending.push(user);
                            }
                        }
                    } else {
                        self.queue_users(node);
                    }
                }
            }
            if self.values[node.index()] == Value::Overdefined
                || self.graph.op(node) == Opcode::Const
            {
                continue;
            }
            let next = self.values[node.index()].join(self.value(node));
            if next != self.values[node.index()] {
                self.values[node.index()] = next;
                self.queue_users(node);
            }
        }
        true
    }
}
