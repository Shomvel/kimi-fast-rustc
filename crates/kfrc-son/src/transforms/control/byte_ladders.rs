//! Dispatch repeated guarded byte equalities once by length. Selected reads
//! retain their original order, result branches and success Phi positions.
//! Collection is bounded and immutable; failure never leaves a partial edit.
use crate::analysis::{
    cfg::ControlFlow,
    dominators::Dominators,
    uses::{Use, UseKind, Uses},
};
use crate::{ComparePredicate, FunctionGraph, GraphLevel, NodeId, Opcode, ValueType};
use kfrc_body::fx_hash::{HashMap, HashSet};
use kfrc_body::{IntegerWidth, RuntimeComparison, TypeOracle, TypeShape};
use std::collections::BTreeMap;
use std::sync::OnceLock;

mod collect;
mod plan;

const MAX_CELLS: usize = 128;
const MAX_WRAPPERS: usize = 16;
const MAX_DEPENDENCIES: usize = 2048;
const MAX_WORK: usize = 50_000;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Stats {
    pub ladders_grouped: usize,
    pub cells_grouped: usize,
    pub length_buckets: usize,
    pub nodes_added: usize,
    pub nodes_retired: usize,
    pub node_visits: usize,
    pub budget_exhausted: bool,
}

struct Budget {
    remaining: usize,
    visits: usize,
    exhausted: bool,
}

impl Budget {
    fn charge(&mut self, count: usize) -> Option<()> {
        self.visits += count.min(self.remaining);
        self.remaining = match self.remaining.checked_sub(count) {
            Some(remaining) => remaining,
            None => {
                self.exhausted = true;
                return None;
            }
        };
        Some(())
    }
}

struct Context<'a> {
    graph: &'a FunctionGraph,
    uses: Uses,
    budget: Budget,
}

impl Context<'_> {
    fn users(&mut self, node: NodeId) -> Option<Vec<Use>> {
        self.budget.charge(1 + self.uses.of(node).len())?;
        Some(self.uses.of(node).to_vec())
    }

    fn input(&self, node: NodeId, slot: usize) -> NodeId {
        self.graph.resolve(self.graph.input(node, slot))
    }

    fn constant(&self, node: NodeId) -> Option<u128> {
        (self.graph.op(node) == Opcode::Const).then(|| self.graph.constant_value(node))
    }

    fn ordinary_int(&self, node: NodeId, width: IntegerWidth, signed: bool) -> bool {
        self.graph.rust_type(node).is_some_and(|ty| {
            self.graph.oracle.scalar_validity(ty).is_none()
                && matches!(self.graph.oracle.shape(ty), TypeShape::Int { width: w, signed: s }
                    if *w == width && *s == signed)
        })
    }

    fn boolean(&self, node: NodeId) -> bool {
        self.graph
            .rust_type(node)
            .is_some_and(|ty| matches!(self.graph.oracle.shape(ty), TypeShape::Bool))
    }

    fn control_user(&mut self, control: NodeId) -> Option<NodeId> {
        let users: Vec<_> = self
            .users(control)?
            .into_iter()
            .filter(|usage| usage.kind == UseKind::Control)
            .map(|usage| usage.user)
            .collect();
        match users.as_slice() {
            [user] => Some(*user),
            _ => None,
        }
    }

    fn projections<const N: usize>(&mut self, source: NodeId) -> Option<[NodeId; N]> {
        let mut projections = [NodeId::NONE; N];
        for usage in self.users(source)? {
            let node = usage.user;
            let slot = self.graph.payload(node) as usize;
            if usage.kind != UseKind::Operand(0)
                || self.graph.op(node) != Opcode::Projection
                || self.graph.inputs(node).len() != 1
                || !self.graph.control(node).is_none()
                || slot >= N
                || !projections[slot].is_none()
            {
                return None;
            }
            projections[slot] = node;
        }
        projections
            .iter()
            .all(|node| !node.is_none())
            .then_some(projections)
    }
}

#[derive(Clone)]
struct Guard {
    branch: NodeId,
    value: NodeId,
    length: u128,
    yes: NodeId,
    no: NodeId,
}

struct Target {
    region: NodeId,
    effect: NodeId,
    slot: usize,
    jump: NodeId,
}

struct Cell {
    guard: Guard,
    entry: NodeId,
    event: NodeId,
    effect: NodeId,
    outgoing: NodeId,
    mismatch: NodeId,
    failure: NodeId,
    failure_effect: NodeId,
    bypass_jump: NodeId,
    mismatch_jump: NodeId,
    success: Target,
    owned: Vec<NodeId>,
}

fn enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var("KFRC_LAYOUT_BYTE_LADDERS").as_deref() != Ok("0"))
}

pub(crate) fn run(graph: &mut FunctionGraph) -> Stats {
    if !enabled() {
        return Stats::default();
    }
    transform(graph, MAX_WORK)
}

fn transform(graph: &mut FunctionGraph, work: usize) -> Stats {
    debug_assert_eq!(graph.level, GraphLevel::Layout);
    let mut context = Context {
        uses: Uses::new(graph),
        graph,
        budget: Budget {
            remaining: work,
            visits: 0,
            exhausted: false,
        },
    };
    let plan = plan::collect(&mut context);
    let mut stats = Stats {
        node_visits: context.budget.visits,
        budget_exhausted: context.budget.exhausted,
        ..Stats::default()
    };
    // One maximal ladder per function bounds construction and avoids using an
    // immutable analysis after any edit. A declined/budgeted probe edits none.
    if !stats.budget_exhausted
        && let Some(plan) = plan
    {
        stats.ladders_grouped = 1;
        stats.cells_grouped = plan.cells.len();
        stats.length_buckets = plan.groups.len();
        stats.nodes_added = 1 + 2 * (plan.groups.len() + 1);
        stats.nodes_retired = plan.retired.len();
        plan.apply(graph);
    }
    stats
}

#[cfg(test)]
mod tests;
