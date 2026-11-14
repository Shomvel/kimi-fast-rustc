//! Find only the values tied to this loop's dynamic iteration. Invariant
//! allocations and projections are shared; LocalSlot identity never changes.
use super::{Limits, Work, plan::Phi};
use crate::analysis::{cfg::ControlFlow, dominators::Dominators};
use crate::{FunctionGraph, NodeId, Opcode};
use kfrc_body::fx_hash::{HashMap, HashSet};
use kfrc_body::{TypeOracle, TypeShape};

pub(super) struct Copies {
    pub nodes: Vec<NodeId>,
    pub slots: Vec<NodeId>,
}

struct Collector<'a> {
    graph: &'a FunctionGraph,
    cfg: &'a ControlFlow,
    dominators: &'a Dominators,
    header: NodeId,
    phis: HashSet<NodeId>,
    aliases: &'a HashMap<NodeId, NodeId>,
    fixed: &'a HashSet<NodeId>,
    memo: HashMap<NodeId, bool>,
    active: HashSet<NodeId>,
    copies: HashSet<NodeId>,
    slots: HashSet<NodeId>,
    limit: usize,
    work: &'a mut Work,
}

#[allow(clippy::too_many_arguments)]
pub(super) fn collect(
    graph: &FunctionGraph,
    cfg: &ControlFlow,
    dominators: &Dominators,
    header: NodeId,
    phis: &[Phi],
    aliases: &HashMap<NodeId, NodeId>,
    fixed: &HashSet<NodeId>,
    roots: &[NodeId],
    limits: Limits,
    work: &mut Work,
) -> Option<Copies> {
    let mut collector = Collector {
        graph,
        cfg,
        dominators,
        header,
        phis: phis.iter().map(|phi| phi.node).collect(),
        aliases,
        fixed,
        memo: HashMap::default(),
        active: HashSet::default(),
        copies: HashSet::default(),
        slots: HashSet::default(),
        limit: limits.template_nodes,
        work,
    };
    for &root in roots {
        collector.visit(root, 0)?;
    }
    let mut nodes: Vec<_> = collector.copies.into_iter().collect();
    let mut slots: Vec<_> = collector.slots.into_iter().collect();
    nodes.sort_unstable();
    slots.sort_unstable();
    Some(Copies { nodes, slots })
}

impl Collector<'_> {
    fn visit(&mut self, source: NodeId, depth: usize) -> Option<bool> {
        self.work.charge(1)?;
        let source = self.graph.resolve(source);
        if source.is_none() {
            return Some(false);
        }
        if source == self.header || self.phis.contains(&source) {
            return Some(true);
        }
        if let Some(&dependent) = self.memo.get(&source) {
            return Some(dependent);
        }
        if depth == 64 {
            self.work.limits_hit += 1;
            return None;
        }
        if !self.active.insert(source) {
            return None;
        }
        let dependent = self.inspect(source, depth)?;
        self.active.remove(&source);
        self.memo.insert(source, dependent);
        Some(dependent)
    }

    fn inspect(&mut self, source: NodeId, depth: usize) -> Option<bool> {
        let graph = self.graph;
        if let Some(&input) = self.aliases.get(&source) {
            return self.visit(input, depth + 1);
        }
        let op = graph.op(source);
        if op == Opcode::IndexAddr && !unambiguous_index(graph, graph.input(source, 1)) {
            return None;
        }
        if op == Opcode::LocalSlot {
            self.slots.insert(source);
            return Some(false);
        }
        if matches!(
            op,
            Opcode::Const | Opcode::Undefined | Opcode::SymbolAddress
        ) {
            return Some(false);
        }
        if op == Opcode::Phi {
            return self
                .dominators
                .dominates(self.cfg, graph.input(source, 0), self.header)
                .then_some(false);
        }
        if op.is_fixed() && !self.fixed.contains(&source) {
            return self
                .dominators
                .dominates(self.cfg, source, self.header)
                .then_some(false);
        }
        if op == Opcode::Projection {
            let parent = graph.input(source, 0);
            if !self.fixed.contains(&parent) {
                return self
                    .dominators
                    .dominates(self.cfg, parent, self.header)
                    .then_some(false);
            }
        } else if !self.fixed.contains(&source) && !graph.control(source).is_none() {
            return None;
        }
        let mut dependent = self.fixed.contains(&source);
        dependent |= self.visit(graph.control(source), depth + 1)?;
        for &input in graph.inputs(source) {
            dependent |= self.visit(input, depth + 1)?;
        }
        if dependent {
            if !self.fixed.contains(&source) && !cloneable(op) {
                return None;
            }
            if self.copies.len() == self.limit {
                self.work.limits_hit += 1;
                return None;
            }
            self.copies.insert(source);
        }
        Some(dependent)
    }
}

fn unambiguous_index(graph: &FunctionGraph, index: NodeId) -> bool {
    let index = graph.resolve(index);
    let Some(ty) = graph.rust_type(index) else {
        return false;
    };
    let (bits, signed) = match graph.oracle.shape(ty) {
        TypeShape::Int { width, signed } => (width.bits(64), *signed),
        TypeShape::Bool => (1, false),
        TypeShape::Char => (32, false),
        _ => return false,
    };
    if bits > 64 {
        return false;
    }
    // S and L currently disagree on dynamic versus literal negative narrow
    // indices. Do not expose that seam by substituting an unrolled constant.
    if graph.op(index) == Opcode::Const {
        let value = graph.constant_value(index);
        value <= u128::from(u64::MAX >> (64 - bits))
            && (!signed || bits == 64 || value < 1u128 << (bits - 1))
    } else {
        !signed || bits == 64
    }
}

fn cloneable(op: Opcode) -> bool {
    matches!(
        op,
        Opcode::Projection
            | Opcode::Add
            | Opcode::Sub
            | Opcode::Mul
            | Opcode::BitAnd
            | Opcode::BitOr
            | Opcode::BitXor
            | Opcode::ShiftLeft
            | Opcode::ShiftRightLogical
            | Opcode::ShiftRightArithmetic
            | Opcode::Negate
            | Opcode::BitNot
            | Opcode::Compare
            | Opcode::IntCast
            | Opcode::MulOverflow
            | Opcode::BitCount
            | Opcode::ByteSwap
            | Opcode::BitReverse
            | Opcode::FunnelShift
            | Opcode::SaturatingAdd
            | Opcode::SaturatingSub
            | Opcode::FieldAddr
            | Opcode::IndexAddr
            | Opcode::PointerCast
    )
}
