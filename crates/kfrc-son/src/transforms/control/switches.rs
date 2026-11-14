//! Collect adjacent integer equality tests after all L control refinement.
//!
//! Plans use one immutable Uses snapshot. Only empty single-entry control
//! wrappers may be bypassed; fixed events and merged values stop a chain.
use crate::analysis::uses::{UseKind, Uses};
use crate::{ComparePredicate, FunctionGraph, GraphLevel, NodeId, Opcode, ValueType};
use kfrc_body::TypeOracle;
use kfrc_body::TypeShape;
use kfrc_body::fx_hash::HashSet;

const MIN_CASES: usize = 4;
const MAX_CASES: usize = 128;
const MAX_WRAPPERS: usize = 16;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct SwitchStats {
    pub switches_created: usize,
    pub cases_merged: usize,
    pub nodes_visited: usize,
}

struct Test {
    value: NodeId,
    bits: u128,
    yes: NodeId,
    no: NodeId,
}

fn equality(graph: &FunctionGraph, branch: NodeId, uses: &Uses) -> Option<Test> {
    let mut condition = graph.resolve(graph.input(branch, 0));
    let mut inverted = false;
    for _ in 0..8 {
        if !graph
            .rust_type(condition)
            .is_some_and(|ty| matches!(graph.oracle.shape(ty), TypeShape::Bool))
        {
            return None;
        }
        if graph.op(condition) != Opcode::BitNot {
            break;
        }
        inverted = !inverted;
        condition = graph.resolve(graph.input(condition, 0));
    }
    if graph.op(condition) != Opcode::Compare {
        return None;
    }
    let equal = match ComparePredicate::from_payload(graph.payload(condition)) {
        ComparePredicate::Equal => true,
        ComparePredicate::NotEqual => false,
        _ => return None,
    } ^ inverted;
    let [left, right] = graph.inputs(condition) else {
        return None;
    };
    let (left, right) = (graph.resolve(*left), graph.resolve(*right));
    let (value, constant) = match (graph.op(left), graph.op(right)) {
        (_, Opcode::Const) => (left, right),
        (Opcode::Const, _) => (right, left),
        _ => return None,
    };
    let ValueType::Rust(ty) = graph.value_type(value) else {
        return None;
    };
    if graph.value_type(constant) != graph.value_type(value) {
        return None;
    }
    let TypeShape::Int { width, .. } = graph.oracle.shape(*ty) else {
        return None;
    };
    let width = width.bits(64);
    if !matches!(width, 8 | 16 | 32 | 64) {
        return None;
    }
    let mut projections = [NodeId::NONE; 2];
    for usage in uses.of(branch) {
        let node = usage.user;
        let slot = graph.payload(node) as usize;
        if usage.kind != UseKind::Operand(0)
            || graph.op(node) != Opcode::Projection
            || graph.value_type(node) != &ValueType::Control
            || slot > 1
            || !projections[slot].is_none()
        {
            return None;
        }
        projections[slot] = node;
    }
    if projections.iter().any(|projection| projection.is_none()) {
        return None;
    }
    let yes = usize::from(!equal);
    Some(Test {
        value,
        bits: graph.constant_value(constant) & ((1u128 << width) - 1),
        yes: projections[yes],
        no: projections[1 - yes],
    })
}

/// The only consumer of each traversed control must be its next event.
/// In particular a Phi or anchored division makes a wrapper nontransparent.
fn continuation(
    graph: &FunctionGraph,
    uses: &Uses,
    mut control: NodeId,
    visits: &mut usize,
) -> Option<(NodeId, Vec<NodeId>)> {
    let mut wrappers = Vec::new();
    for _ in 0..=MAX_WRAPPERS {
        *visits += 1;
        let [usage] = uses.of(control) else {
            return None;
        };
        let next = usage.user;
        if usage.kind != UseKind::Control {
            return None;
        }
        if graph.op(next) == Opcode::If {
            return Some((next, wrappers));
        }
        if graph.op(next) != Opcode::Jump || !graph.inputs(next).is_empty() {
            return None;
        }
        let [usage] = uses.of(next) else {
            return None;
        };
        let region = usage.user;
        if usage.kind != UseKind::Operand(0)
            || graph.op(region) != Opcode::Region
            || graph.inputs(region) != [next]
        {
            return None;
        }
        wrappers.extend([next, region]);
        control = region;
    }
    None
}

struct Plan {
    control: NodeId,
    value: NodeId,
    branches: Vec<NodeId>,
    cases: Vec<u128>,
    exits: Vec<NodeId>,
    retired: Vec<NodeId>,
}

impl Plan {
    fn collect(
        graph: &FunctionGraph,
        uses: &Uses,
        root: NodeId,
        claimed: &HashSet<NodeId>,
        visits: &mut usize,
    ) -> Option<Self> {
        let first = equality(graph, root, uses)?;
        let mut no = first.no;
        let mut plan = Self {
            control: graph.control(root),
            value: first.value,
            branches: vec![root],
            cases: vec![first.bits],
            exits: vec![first.yes],
            retired: Vec::new(),
        };
        let mut seen = HashSet::default();
        seen.insert(root);
        while plan.cases.len() < MAX_CASES {
            let Some((branch, wrappers)) = continuation(graph, uses, no, visits) else {
                break;
            };
            if claimed.contains(&branch) || !seen.insert(branch) {
                break;
            }
            *visits += 1;
            let Some(test) = equality(graph, branch, uses) else {
                break;
            };
            if test.value != plan.value || plan.cases.contains(&test.bits) {
                break;
            }
            plan.retired.push(no);
            plan.retired.extend(wrappers);
            plan.branches.push(branch);
            plan.cases.push(test.bits);
            plan.exits.push(test.yes);
            no = test.no;
        }
        plan.exits.push(no);
        (plan.cases.len() >= MIN_CASES).then_some(plan)
    }

    fn apply(self, graph: &mut FunctionGraph) {
        let payload = graph.integer_switches.len() as u32;
        let count = self.cases.len() + 1;
        graph.integer_switches.push(self.cases);
        let switch = graph.add_node(
            Opcode::Switch,
            self.control,
            &[self.value],
            payload,
            ValueType::Tuple(vec![ValueType::Control; count]),
        );
        for (slot, old) in self.exits.into_iter().enumerate() {
            let projection = graph.add_node(
                Opcode::Projection,
                NodeId::NONE,
                &[switch],
                slot as u32,
                ValueType::Control,
            );
            graph.replace(old, projection);
        }
        for node in self.branches.into_iter().chain(self.retired) {
            graph.remove_node(node);
        }
    }
}

pub(crate) fn run(graph: &mut FunctionGraph) -> SwitchStats {
    debug_assert_eq!(graph.level, GraphLevel::Layout);
    graph.resolve_edges();
    let mut stats = SwitchStats::default();
    let candidates: Vec<_> = graph
        .ids()
        .filter(|node| graph.op(*node) == Opcode::If)
        .collect();
    if candidates.len() < MIN_CASES {
        return stats;
    }
    let uses = Uses::new(graph);
    let mut claimed = HashSet::default();
    let mut plans = Vec::new();
    for root in candidates {
        stats.nodes_visited += 1;
        if claimed.contains(&root) {
            continue;
        }
        if let Some(plan) = Plan::collect(graph, &uses, root, &claimed, &mut stats.nodes_visited) {
            claimed.extend(plan.branches.iter().copied());
            stats.cases_merged += plan.cases.len();
            plans.push(plan);
        }
    }
    stats.switches_created = plans.len();
    for plan in plans {
        plan.apply(graph);
    }
    if stats.switches_created != 0 {
        graph.compact();
    }
    stats
}

#[cfg(test)]
mod tests;
