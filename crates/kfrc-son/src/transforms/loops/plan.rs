use super::{Limits, Work, count, values};
use crate::analysis::{
    cfg::{ControlFlow, is_anchored_value},
    dominators::Dominators,
    loops::NaturalLoop,
    uses::Uses,
};
use crate::{FunctionGraph, NodeId, Opcode, ValueType};
use kfrc_body::fx_hash::{HashMap, HashSet};
use kfrc_body::{TypeOracle, TypeShape};

pub(super) struct Phi {
    pub node: NodeId,
    pub entry: NodeId,
    pub backedge: NodeId,
}

pub(super) struct Plan {
    pub header: NodeId,
    pub branch: NodeId,
    pub exit: NodeId,
    pub latch: NodeId,
    pub entry_slot: usize,
    pub phis: Vec<Phi>,
    pub induction: count::Induction,
    pub aliases: HashMap<NodeId, NodeId>,
    pub copies: Vec<NodeId>,
    pub slots: Vec<NodeId>,
    pub consumers: Vec<NodeId>,
}

pub(super) fn scalar(graph: &FunctionGraph, node: NodeId) -> bool {
    graph
        .rust_type(node)
        .is_some_and(|ty| match graph.oracle.shape(ty) {
            TypeShape::Int { width, .. } => width.bits(64) <= 64,
            TypeShape::Bool | TypeShape::Char => true,
            _ => false,
        })
}

fn scalar_or_effect(graph: &FunctionGraph, node: NodeId) -> bool {
    graph.value_type(node) == &ValueType::Effect || scalar(graph, node)
}

fn fixed_allowed(graph: &FunctionGraph, node: NodeId) -> bool {
    match graph.op(node) {
        Opcode::Opaque | Opcode::RequireDefined | Opcode::Assume => {
            scalar(graph, graph.input(node, 1))
        }
        Opcode::StoreValue => scalar(graph, graph.input(node, 2)),
        Opcode::LoadValue => match graph.value_type(node) {
            ValueType::Tuple(results) => match results.get(2) {
                Some(ValueType::Rust(ty)) => match graph.oracle.shape(*ty) {
                    TypeShape::Int { width, .. } => width.bits(64) <= 64,
                    TypeShape::Bool | TypeShape::Char => true,
                    _ => false,
                },
                _ => false,
            },
            _ => false,
        },
        _ => false,
    }
}

impl Plan {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        graph: &FunctionGraph,
        cfg: &ControlFlow,
        dominators: &Dominators,
        uses: &Uses,
        nodes: &[NodeId],
        loop_: &NaturalLoop,
        limits: Limits,
        work: &mut Work,
    ) -> Option<Self> {
        work.charge(loop_.members.len())?;
        let header = loop_.header;
        if loop_.contains_irreducible
            || loop_.parent.is_some()
            || loop_.latches.len() != 1
            || loop_.entry_edges.len() != 1
            || loop_.exit_edges.len() != 1
            || graph.op(header) != Opcode::Region
            || graph.inputs(header).len() != 2
        {
            return None;
        }
        let latch = loop_.latches[0];
        if graph.op(latch) != Opcode::Jump || cfg.successors(latch) != [header] {
            return None;
        }
        let entry = loop_.entry_edges[0];
        if entry.to != header {
            return None;
        }
        let entry_slot = graph
            .inputs(header)
            .iter()
            .position(|&node| node == entry.from)?;
        if graph.input(header, 1 - entry_slot) != latch {
            return None;
        }
        let &[branch] = cfg.successors(header) else {
            return None;
        };
        if graph.op(branch) != Opcode::If || graph.control(branch) != header {
            return None;
        }
        let branches = cfg.successors(branch);
        if branches.len() != 2 {
            return None;
        }
        let body = *branches.iter().find(|&&node| loop_.contains(node))?;
        let exit = *branches.iter().find(|&&node| !loop_.contains(node))?;
        if [body, exit]
            .into_iter()
            .any(|node| graph.op(node) != Opcode::Projection || graph.input(node, 0) != branch)
            || graph.payload(body) == graph.payload(exit)
            || loop_.exit_edges[0].from != branch
            || loop_.exit_edges[0].to != exit
        {
            return None;
        }
        let mut aliases = HashMap::from_iter([(body, header)]);
        let mut fixed = HashSet::default();
        let mut path = HashSet::from_iter([header, branch]);
        let mut at = body;
        loop {
            work.charge(1)?;
            if !loop_.contains(at) || !path.insert(at) {
                return None;
            }
            if at == latch {
                aliases.insert(at, graph.control(at));
                break;
            }
            match graph.op(at) {
                Opcode::Projection if at == body => {}
                Opcode::Projection if fixed.contains(&graph.input(at, 0)) => {}
                Opcode::Jump => {
                    aliases.insert(at, graph.control(at));
                }
                Opcode::Region if graph.inputs(at).len() == 1 => {
                    aliases.insert(at, graph.input(at, 0));
                }
                _ if fixed_allowed(graph, at) => {
                    fixed.insert(at);
                }
                _ => return None,
            }
            let &[next] = cfg.successors(at) else {
                return None;
            };
            at = next;
        }
        if path.len() != loop_.members.len() {
            return None;
        }
        let mut phis = Vec::new();
        for &node in nodes {
            work.charge(1)?;
            if is_anchored_value(graph.op(node)) && loop_.contains(graph.control(node)) {
                return None;
            }
            if graph.op(node) != Opcode::Phi {
                continue;
            }
            let region = graph.input(node, 0);
            if !loop_.contains(region) {
                continue;
            }
            if !scalar_or_effect(graph, node) {
                return None;
            }
            if region == header && graph.inputs(node).len() == 3 {
                phis.push(Phi {
                    node,
                    entry: graph.input(node, entry_slot + 1),
                    backedge: graph.input(node, 2 - entry_slot),
                });
            } else if region != header && graph.inputs(node).len() == 2 {
                aliases.insert(node, graph.input(node, 1));
            } else {
                return None;
            }
        }
        let induction = count::prove(
            graph,
            branch,
            body,
            &phis,
            entry_slot,
            limits.iterations,
            work,
        )?;
        let mut roots: Vec<_> = fixed.iter().copied().collect();
        roots.push(latch);
        roots.extend(
            phis.iter()
                .filter(|phi| phi.node != induction.phi)
                .map(|phi| phi.backedge),
        );
        let copies = values::collect(
            graph, cfg, dominators, header, &phis, &aliases, &fixed, &roots, limits, work,
        )?;
        let consumers = phis
            .iter()
            .flat_map(|phi| uses.of(phi.node).iter().map(|usage| usage.user))
            .collect();
        Some(Self {
            header,
            branch,
            exit,
            latch,
            entry_slot,
            phis,
            induction,
            aliases,
            copies: copies.nodes,
            slots: copies.slots,
            consumers,
        })
    }

    pub fn cost(&self) -> usize {
        let count = self.induction.values.len() - 1;
        count * (self.copies.len() + 1)
    }
}
