//! Replace a repeated wrapping integer product with an exact modular AddRec:
//! `c * {start,+,step}` becomes `{c*start,+,c*step}` at the same header.
//! No control, effect, memory access, observation or pointer recurrence moves.
use crate::analysis::{
    cfg::ControlFlow,
    induction::{Induction, constant},
    loops::NaturalLoop,
    uses::{UseKind, Uses},
};
use crate::{FunctionGraph, GraphLevel, NodeId, Opcode, ValueType};
use kfrc_body::fx_hash::HashSet;
use kfrc_body::{TypeOracle, TypeShape};

#[derive(Default)]
pub(crate) struct StrengthStats {
    pub products_reduced: usize,
    pub recurrences_created: usize,
    pub nodes_added: usize,
    pub analysis_visits: usize,
    pub budget_bailouts: usize,
    pub dirty_nodes: Vec<NodeId>,
}

struct Work {
    left: usize,
    visits: usize,
    exhausted: bool,
}

impl Work {
    fn charge(&mut self, count: usize) -> Option<()> {
        let paid = self.left.min(count);
        self.left -= paid;
        self.visits += paid;
        if paid == count {
            Some(())
        } else {
            self.exhausted = true;
            None
        }
    }
}

struct Product {
    node: NodeId,
    induction: Induction,
    factor: u64,
    header: NodeId,
    entry: usize,
}

/// x86 already handles powers of two and 3/5/9 cheaply. Creating another live
/// recurrence for those factors would not remove an expensive operation.
fn worthwhile(factor: u64) -> bool {
    factor > 1 && !factor.is_power_of_two() && !matches!(factor, 3 | 5 | 9)
}

fn product(graph: &FunctionGraph, node: NodeId) -> Option<(NodeId, u64)> {
    if graph.op(node) != Opcode::Mul || graph.payload(node) != 0 || !graph.control(node).is_none() {
        return None;
    }
    let ty = graph.rust_type(node)?;
    let TypeShape::Int { width, .. } = graph.oracle.shape(ty) else {
        return None;
    };
    let bits = width.bits(64);
    if bits > 64 {
        return None;
    }
    let mask = u64::MAX >> (64 - bits);
    let a = graph.resolve(graph.input(node, 0));
    let b = graph.resolve(graph.input(node, 1));
    let (phi, factor) = if graph.op(a) == Opcode::Phi {
        (a, constant(graph, b)? & mask)
    } else {
        (b, constant(graph, a)? & mask)
    };
    (graph.op(phi) == Opcode::Phi && graph.rust_type(phi) == Some(ty) && worthwhile(factor))
        .then_some((phi, factor))
}

/// Exit-only computations already sink out of the loop in GCM. Follow bounded
/// floating uses to find an operation that really consumes this value inside
/// the loop; a Phi consumes an operand on that operand's incoming edge.
fn repeated_use(
    graph: &FunctionGraph,
    uses: &Uses,
    loop_: &NaturalLoop,
    source: NodeId,
    work: &mut Work,
) -> Option<bool> {
    let mut queue = vec![source];
    let mut seen = HashSet::default();
    while let Some(node) = queue.pop() {
        work.charge(1)?;
        if !seen.insert(node) {
            continue;
        }
        if seen.len() > 64 {
            return Some(false);
        }
        for usage in uses.of(node) {
            work.charge(1)?;
            let user = usage.user;
            if graph.op(user) == Opcode::Phi {
                if let UseKind::Operand(slot) = usage.kind()
                    && slot != 0
                    && loop_.contains(graph.input(graph.input(user, 0), slot - 1))
                {
                    return Some(true);
                }
            } else if graph.op(user).is_fixed() || !graph.control(user).is_none() {
                if loop_.contains(graph.control(user)) {
                    return Some(true);
                }
            } else {
                queue.push(user);
            }
        }
    }
    Some(false)
}

pub(crate) fn run(graph: &mut FunctionGraph) -> StrengthStats {
    run_with_budget(graph, 32_768)
}

pub(super) fn run_with_budget(graph: &mut FunctionGraph, budget: usize) -> StrengthStats {
    let mut stats = StrengthStats::default();
    if graph.level != GraphLevel::Specialized {
        return stats;
    }
    graph.resolve_edges();
    let mut work = Work {
        left: budget,
        visits: 0,
        exhausted: false,
    };
    let mut candidates = Vec::new();
    for node in graph.ids() {
        if work.charge(1 + graph.inputs(node).len()).is_none() {
            break;
        }
        if let Some((phi, factor)) = product(graph, node) {
            candidates.push((node, phi, factor));
        }
    }
    let mut plans = Vec::new();
    if !work.exhausted
        && !candidates.is_empty()
        && work.charge(graph.len().saturating_mul(3)).is_some()
    {
        let cfg = ControlFlow::new(graph);
        let loops = cfg.loop_info();
        let uses = Uses::new(graph);
        let loop_stats = loops.stats();
        if work
            .charge(loop_stats.member_edge_visits + loop_stats.scc_edge_visits)
            .is_some()
        {
            for (node, phi, factor) in candidates {
                if work.charge(1).is_none() {
                    break;
                }
                let header = graph.input(phi, 0);
                let Some(loop_) = loops.loop_by_header(header) else {
                    continue;
                };
                if loop_.contains_irreducible
                    || loop_.entry_edges.len() != 1
                    || loop_.latches.len() != 1
                    || graph.op(header) != Opcode::Region
                    || graph.inputs(header).len() != 2
                {
                    continue;
                }
                let Some(entry) = graph
                    .inputs(header)
                    .iter()
                    .position(|&pred| pred == loop_.entry_edges[0].from)
                else {
                    continue;
                };
                if graph.input(header, 1 - entry) != loop_.latches[0] {
                    continue;
                }
                let Some(induction) = Induction::analyze(graph, phi, entry) else {
                    continue;
                };
                if induction.step == 0
                    || repeated_use(graph, &uses, loop_, node, &mut work) != Some(true)
                {
                    continue;
                }
                if plans.len() == 16 {
                    stats.budget_bailouts += 1;
                    break;
                }
                plans.push(Product {
                    node,
                    induction,
                    factor,
                    header,
                    entry,
                });
            }
        }
        if !work.exhausted {
            // The proof and use index refer to the immutable snapshot. NodeIds
            // stay stable while publishing these independent recurrences.
            let before = graph.len();
            for plan in plans {
                let ty = ValueType::Rust(plan.induction.ty);
                let factor = graph.add_constant(u128::from(plan.factor), plan.induction.ty);
                let initial = graph.add_node(
                    Opcode::Mul,
                    NodeId::NONE,
                    &[plan.induction.initial, factor],
                    0,
                    ty.clone(),
                );
                let delta = plan.factor.wrapping_mul(plan.induction.step) & plan.induction.mask;
                let replacement = if delta == 0 {
                    initial
                } else {
                    let recurrence = graph.add_node(Opcode::Phi, NodeId::NONE, &[], 0, ty.clone());
                    let step = graph.add_constant(u128::from(delta), plan.induction.ty);
                    let next =
                        graph.add_node(Opcode::Add, NodeId::NONE, &[recurrence, step], 0, ty);
                    let mut inputs = [plan.header, next, next];
                    inputs[plan.entry + 1] = initial;
                    graph.set_inputs(recurrence, &inputs);
                    stats.recurrences_created += 1;
                    recurrence
                };
                stats
                    .dirty_nodes
                    .extend(uses.of(plan.node).iter().map(|usage| usage.user));
                graph.replace(plan.node, replacement);
                stats.products_reduced += 1;
            }
            stats.nodes_added = graph.len() - before;
            stats
                .dirty_nodes
                .extend((before..graph.len()).map(|id| NodeId(id as u32)));
            graph.resolve_edges();
        }
    }
    stats.analysis_visits = work.visits;
    stats.budget_bailouts += usize::from(work.exhausted);
    stats
}
