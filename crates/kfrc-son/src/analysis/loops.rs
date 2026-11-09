//! Natural loops over one immutable reachable control-flow snapshot.
//!
//! These are structural facts, not trip-count, no-wrap, or termination proofs.
//! Rebuild after every CFG edit or compaction, just like `Dominators`.
use super::{cfg::ControlFlow, dominators::Dominators};
use crate::NodeId;
use std::collections::{BTreeSet, HashMap};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ControlEdge {
    pub from: NodeId,
    pub to: NodeId,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NaturalLoop {
    pub header: NodeId,
    /// Sources of all edges to this header dominated by the header.
    pub latches: Vec<NodeId>,
    /// Sorted by NodeId, including the header and nested-loop members.
    pub members: Vec<NodeId>,
    pub entry_edges: Vec<ControlEdge>,
    pub exit_edges: Vec<ControlEdge>,
    /// Header of the smallest strictly enclosing natural loop.
    pub parent: Option<NodeId>,
    /// Outermost loops have depth one.
    pub depth: usize,
    /// A consumer requiring a reducible loop must reject this loop.
    pub contains_irreducible: bool,
}

impl NaturalLoop {
    pub fn contains(&self, node: NodeId) -> bool {
        self.members.binary_search(&node).is_ok()
    }
}

/// A cyclic SCC remaining after all dominance backedges are removed. This also
/// exposes irreducible subregions nested inside an otherwise natural outer loop.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IrreducibleRegion {
    pub members: Vec<NodeId>,
    pub entry_edges: Vec<ControlEdge>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LoopStats {
    pub control_nodes: usize,
    pub control_edges: usize,
    pub natural_backedges: usize,
    pub member_edge_visits: usize,
    pub scc_edge_visits: usize,
}

#[derive(Clone, Debug)]
pub struct LoopInfo {
    loops: Vec<NaturalLoop>,
    headers: HashMap<NodeId, usize>,
    innermost: HashMap<NodeId, usize>,
    irreducible: Vec<IrreducibleRegion>,
    irreducible_nodes: BTreeSet<NodeId>,
    stats: LoopStats,
}

impl LoopInfo {
    /// `cfg` and `dominators` must describe the same unmodified graph.
    pub fn new(cfg: &ControlFlow, dominators: &Dominators) -> Self {
        let count = cfg.order.len();
        let mut stats = LoopStats {
            control_nodes: count,
            control_edges: cfg.predecessors.iter().map(Vec::len).sum(),
            ..LoopStats::default()
        };
        let mut backedges = vec![Vec::new(); count];
        let mut remaining = vec![Vec::new(); count];
        let mut reverse_remaining = vec![Vec::new(); count];
        for (to, predecessors) in cfg.predecessors.iter().enumerate() {
            for &from in predecessors {
                if dominators.dominates_position(to, from) {
                    backedges[to].push(from);
                    stats.natural_backedges += 1;
                } else {
                    remaining[from].push(to);
                    reverse_remaining[to].push(from);
                }
            }
        }
        let mut irreducible = Vec::new();
        let mut irreducible_nodes = BTreeSet::new();
        for component in cyclic_components(&remaining, &reverse_remaining, &mut stats) {
            let (entry_edges, _) = boundary_edges(cfg, &component);
            let members = node_ids(cfg, &component);
            irreducible_nodes.extend(members.iter().copied());
            irreducible.push(IrreducibleRegion {
                members,
                entry_edges,
            });
        }
        irreducible.sort_by_key(|region| region.members[0]);
        let mut loops = Vec::new();
        for (header, latches) in backedges.iter().enumerate() {
            if latches.is_empty() {
                continue;
            }
            let mut members = BTreeSet::from([header]);
            let mut pending = latches.clone();
            while let Some(node) = pending.pop() {
                if !members.insert(node) {
                    continue;
                }
                stats.member_edge_visits += cfg.predecessors[node].len();
                pending.extend_from_slice(&cfg.predecessors[node]);
            }
            let (entry_edges, exit_edges) = boundary_edges(cfg, &members);
            let members = node_ids(cfg, &members);
            let contains_irreducible = members.iter().any(|node| irreducible_nodes.contains(node));
            let mut latches: Vec<_> = latches.iter().map(|&node| cfg.order[node]).collect();
            latches.sort_unstable();
            latches.dedup();
            loops.push(NaturalLoop {
                header: cfg.order[header],
                latches,
                members,
                entry_edges,
                exit_edges,
                parent: None,
                depth: 1,
                contains_irreducible,
            });
        }
        let parents: Vec<_> = loops
            .iter()
            .map(|child| {
                loops
                    .iter()
                    .enumerate()
                    .filter(|(_, parent)| {
                        parent.members.len() > child.members.len()
                            && child.members.iter().all(|&node| parent.contains(node))
                    })
                    .min_by_key(|(_, parent)| parent.members.len())
                    .map(|(index, _)| index)
            })
            .collect();
        for index in 0..loops.len() {
            loops[index].parent = parents[index].map(|parent| loops[parent].header);
            let mut parent = parents[index];
            while let Some(outer) = parent {
                loops[index].depth += 1;
                parent = parents[outer];
            }
        }
        let headers = loops
            .iter()
            .enumerate()
            .map(|(index, loop_)| (loop_.header, index))
            .collect();
        let mut innermost: HashMap<NodeId, usize> = HashMap::new();
        for (index, loop_) in loops.iter().enumerate() {
            for &node in &loop_.members {
                let current = innermost.entry(node).or_insert(index);
                if loops[*current].members.len() > loop_.members.len() {
                    *current = index;
                }
            }
        }
        Self {
            loops,
            headers,
            innermost,
            irreducible,
            irreducible_nodes,
            stats,
        }
    }

    pub fn loops(&self) -> &[NaturalLoop] {
        &self.loops
    }

    pub fn loop_by_header(&self, header: NodeId) -> Option<&NaturalLoop> {
        self.headers.get(&header).map(|&index| &self.loops[index])
    }

    /// Smallest containing natural loop. Check `contains_irreducible` before
    /// using its nesting as a reducible-loop proof; no IV facts are implied.
    pub fn loop_for(&self, node: NodeId) -> Option<&NaturalLoop> {
        self.innermost.get(&node).map(|&index| &self.loops[index])
    }

    pub fn irreducible_regions(&self) -> &[IrreducibleRegion] {
        &self.irreducible
    }

    pub fn is_irreducible(&self, node: NodeId) -> bool {
        self.irreducible_nodes.contains(&node)
    }

    pub fn stats(&self) -> LoopStats {
        self.stats
    }
}

fn node_ids(cfg: &ControlFlow, positions: &BTreeSet<usize>) -> Vec<NodeId> {
    let mut nodes: Vec<_> = positions
        .iter()
        .map(|&position| cfg.order[position])
        .collect();
    nodes.sort_unstable();
    nodes
}

fn boundary_edges(
    cfg: &ControlFlow,
    members: &BTreeSet<usize>,
) -> (Vec<ControlEdge>, Vec<ControlEdge>) {
    let mut entries = Vec::new();
    let mut exits = Vec::new();
    for &node in members {
        for &predecessor in &cfg.predecessors[node] {
            if !members.contains(&predecessor) {
                entries.push(ControlEdge {
                    from: cfg.order[predecessor],
                    to: cfg.order[node],
                });
            }
        }
        for &successor in cfg.successors(cfg.order[node]) {
            if !members.contains(&cfg.position[&successor]) {
                exits.push(ControlEdge {
                    from: cfg.order[node],
                    to: successor,
                });
            }
        }
    }
    entries.sort_unstable();
    entries.dedup();
    exits.sort_unstable();
    exits.dedup();
    (entries, exits)
}

/// Iterative Kosaraju avoids Rust stack growth on large or irreducible CFGs.
fn cyclic_components(
    successors: &[Vec<usize>],
    predecessors: &[Vec<usize>],
    stats: &mut LoopStats,
) -> Vec<BTreeSet<usize>> {
    let mut visited = vec![false; successors.len()];
    let mut postorder = Vec::new();
    for root in 0..successors.len() {
        if visited[root] {
            continue;
        }
        visited[root] = true;
        let mut stack = vec![(root, 0)];
        while let Some((node, next)) = stack.pop() {
            if let Some(&successor) = successors[node].get(next) {
                stats.scc_edge_visits += 1;
                stack.push((node, next + 1));
                if !visited[successor] {
                    visited[successor] = true;
                    stack.push((successor, 0));
                }
            } else {
                postorder.push(node);
            }
        }
    }
    visited.fill(false);
    let mut components = Vec::new();
    for root in postorder.into_iter().rev() {
        if visited[root] {
            continue;
        }
        let mut members = BTreeSet::new();
        let mut pending = vec![root];
        visited[root] = true;
        while let Some(node) = pending.pop() {
            members.insert(node);
            for &predecessor in &predecessors[node] {
                stats.scc_edge_visits += 1;
                if !visited[predecessor] {
                    visited[predecessor] = true;
                    pending.push(predecessor);
                }
            }
        }
        if members.len() > 1 || successors[root].contains(&root) {
            components.push(members);
        }
    }
    components
}
