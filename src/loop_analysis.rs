//! Dominance and natural-loop analysis for target-independent control-flow graphs.
//!
//! Only blocks reachable from the entry participate in dominance and loop
//! discovery. Unreachable blocks are reported separately. Loop-invariant motion
//! uses stable, dominance-checked reordering without instruction-level
//! parallelism scheduling.

use std::collections::{BTreeMap, BTreeSet};

use crate::cfg::{BasicBlock, BlockId, ControlFlowGraph};
use crate::son::{BinaryOp, Graph, InputChannel, NodeId, NodeKind};

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct BackEdge {
    pub from: BlockId,
    pub to: BlockId,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NaturalLoop {
    pub header: BlockId,
    pub blocks: BTreeSet<BlockId>,
    pub back_edges: Vec<BackEdge>,
    pub parent: Option<BlockId>,
    pub depth: usize,
}

impl NaturalLoop {
    pub fn contains(&self, block: BlockId) -> bool {
        self.blocks.contains(&block)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LoopAnalysis {
    pub dominators: BTreeMap<BlockId, BTreeSet<BlockId>>,
    pub immediate_dominators: BTreeMap<BlockId, BlockId>,
    pub dominator_tree: BTreeMap<BlockId, Vec<BlockId>>,
    pub back_edges: Vec<BackEdge>,
    pub loops: Vec<NaturalLoop>,
    pub unreachable_blocks: BTreeSet<BlockId>,
    pub irreducible: bool,
}

impl LoopAnalysis {
    pub fn dominates(&self, dominator: BlockId, block: BlockId) -> bool {
        self.dominators
            .get(&block)
            .is_some_and(|dominators| dominators.contains(&dominator))
    }

    pub fn dominators(&self, block: BlockId) -> Option<&BTreeSet<BlockId>> {
        self.dominators.get(&block)
    }

    pub fn immediate_dominator(&self, block: BlockId) -> Option<BlockId> {
        self.immediate_dominators.get(&block).copied()
    }

    pub fn dominator_children(&self, block: BlockId) -> Option<&[BlockId]> {
        self.dominator_tree.get(&block).map(Vec::as_slice)
    }

    pub fn loop_for_header(&self, header: BlockId) -> Option<&NaturalLoop> {
        self.loops
            .iter()
            .find(|natural_loop| natural_loop.header == header)
    }

    pub fn innermost_loop(&self, block: BlockId) -> Option<&NaturalLoop> {
        self.loops
            .iter()
            .filter(|natural_loop| natural_loop.contains(block))
            .min_by_key(|natural_loop| (natural_loop.blocks.len(), natural_loop.header))
    }

    pub fn is_irreducible(&self) -> bool {
        self.irreducible
    }
}

impl<V> ControlFlowGraph<V> {
    pub fn loop_analysis(&self) -> LoopAnalysis {
        analyze_loops(self)
    }
}

pub fn analyze_loops<V>(graph: &ControlFlowGraph<V>) -> LoopAnalysis {
    let dominators = compute_dominators(graph);
    let immediate_dominators = compute_immediate_dominators(graph, &dominators);
    let dominator_tree = build_dominator_tree(graph, &immediate_dominators);
    let back_edges = find_back_edges(graph, &dominators);
    let loops = build_natural_loops(graph, &back_edges);
    let reachable = graph
        .reverse_postorder()
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    let unreachable_blocks = graph
        .iter()
        .map(|block| block.id)
        .filter(|id| !reachable.contains(id))
        .collect();
    let irreducible = has_irreducible_region(graph, &reachable);

    LoopAnalysis {
        dominators,
        immediate_dominators,
        dominator_tree,
        back_edges,
        loops,
        unreachable_blocks,
        irreducible,
    }
}

pub fn compute_dominators<V>(graph: &ControlFlowGraph<V>) -> BTreeMap<BlockId, BTreeSet<BlockId>> {
    let reachable = graph
        .reverse_postorder()
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    let mut dominators = reachable
        .iter()
        .map(|&id| (id, reachable.clone()))
        .collect::<BTreeMap<_, _>>();
    dominators.insert(graph.entry(), BTreeSet::from([graph.entry()]));

    loop {
        let mut changed = false;
        for &id in graph.reverse_postorder() {
            if id == graph.entry() {
                continue;
            }

            let mut predecessors = graph
                .predecessors(id)
                .expect("graph blocks have predecessors")
                .iter()
                .filter(|predecessor| reachable.contains(predecessor));
            let first = predecessors
                .next()
                .expect("a reachable non-entry block has a reachable predecessor");
            let mut new_dominators = dominators[first].clone();
            for predecessor in predecessors {
                new_dominators = new_dominators
                    .intersection(&dominators[predecessor])
                    .copied()
                    .collect();
            }
            new_dominators.insert(id);
            if dominators.get(&id) != Some(&new_dominators) {
                dominators.insert(id, new_dominators);
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }

    dominators
}

fn compute_immediate_dominators<V>(
    graph: &ControlFlowGraph<V>,
    dominators: &BTreeMap<BlockId, BTreeSet<BlockId>>,
) -> BTreeMap<BlockId, BlockId> {
    let mut immediate = BTreeMap::new();
    for &id in graph.reverse_postorder() {
        if id == graph.entry() {
            continue;
        }
        let strict = dominators[&id]
            .iter()
            .copied()
            .filter(|&dominator| dominator != id)
            .collect::<BTreeSet<_>>();
        let idom = strict
            .iter()
            .copied()
            .find(|&candidate| {
                strict
                    .iter()
                    .filter(|&&other| other != candidate)
                    .all(|other| dominators[&candidate].contains(other))
            })
            .expect("every reachable non-entry block has an immediate dominator");
        immediate.insert(id, idom);
    }
    immediate
}

fn build_dominator_tree<V>(
    graph: &ControlFlowGraph<V>,
    immediate_dominators: &BTreeMap<BlockId, BlockId>,
) -> BTreeMap<BlockId, Vec<BlockId>> {
    let mut tree = graph
        .reverse_postorder()
        .iter()
        .map(|&id| (id, Vec::new()))
        .collect::<BTreeMap<_, _>>();
    for (&block, &parent) in immediate_dominators {
        tree.get_mut(&parent)
            .expect("an immediate dominator is reachable")
            .push(block);
    }
    tree
}

pub fn find_back_edges<V>(
    graph: &ControlFlowGraph<V>,
    dominators: &BTreeMap<BlockId, BTreeSet<BlockId>>,
) -> Vec<BackEdge> {
    let mut edges = BTreeSet::new();
    for &from in graph.reverse_postorder() {
        for &to in graph.successors(from).expect("RPO contains graph blocks") {
            if dominators
                .get(&from)
                .is_some_and(|block_dominators| block_dominators.contains(&to))
            {
                edges.insert(BackEdge { from, to });
            }
        }
    }
    edges.into_iter().collect()
}

fn build_natural_loops<V>(
    graph: &ControlFlowGraph<V>,
    back_edges: &[BackEdge],
) -> Vec<NaturalLoop> {
    let mut by_header = BTreeMap::<BlockId, NaturalLoop>::new();
    for &edge in back_edges {
        let natural_loop = by_header.entry(edge.to).or_insert_with(|| NaturalLoop {
            header: edge.to,
            blocks: BTreeSet::new(),
            back_edges: Vec::new(),
            parent: None,
            depth: 0,
        });
        natural_loop.back_edges.push(edge);
        natural_loop.blocks.insert(edge.to);
        natural_loop.blocks.insert(edge.from);

        let mut pending = if edge.from == edge.to {
            Vec::new()
        } else {
            vec![edge.from]
        };
        while let Some(block) = pending.pop() {
            for &predecessor in graph
                .predecessors(block)
                .expect("natural-loop blocks belong to the graph")
            {
                if predecessor != edge.to
                    && graph.is_reachable(predecessor)
                    && natural_loop.blocks.insert(predecessor)
                {
                    pending.push(predecessor);
                }
            }
        }
    }

    let mut loops = by_header.into_values().collect::<Vec<_>>();
    for index in 0..loops.len() {
        loops[index].parent = loops
            .iter()
            .enumerate()
            .filter(|&(parent_index, parent)| {
                parent_index != index
                    && parent.blocks.len() > loops[index].blocks.len()
                    && parent.blocks.is_superset(&loops[index].blocks)
            })
            .min_by_key(|(_, parent)| (parent.blocks.len(), parent.header))
            .map(|(_, parent)| parent.header);
    }

    let headers = loops
        .iter()
        .enumerate()
        .map(|(index, natural_loop)| (natural_loop.header, index))
        .collect::<BTreeMap<_, _>>();
    for index in 0..loops.len() {
        let mut depth = 0;
        let mut parent = loops[index].parent;
        while let Some(header) = parent {
            depth += 1;
            parent = loops[headers[&header]].parent;
        }
        loops[index].depth = depth;
    }
    loops
}

fn has_irreducible_region<V>(graph: &ControlFlowGraph<V>, reachable: &BTreeSet<BlockId>) -> bool {
    let mut successors = reachable
        .iter()
        .map(|&id| {
            let reduced_successors = graph
                .successors(id)
                .expect("reachable IDs are graph blocks")
                .iter()
                .copied()
                .filter(|successor| successor != &id && reachable.contains(successor))
                .collect::<BTreeSet<_>>();
            (id, reduced_successors)
        })
        .collect::<BTreeMap<_, _>>();

    loop {
        let mut predecessors = successors
            .keys()
            .map(|&id| (id, BTreeSet::new()))
            .collect::<BTreeMap<_, _>>();
        for (&from, targets) in &successors {
            for &to in targets {
                predecessors
                    .get_mut(&to)
                    .expect("reduction preserves reachable nodes")
                    .insert(from);
            }
        }

        let collapse = predecessors.iter().find_map(|(&id, sources)| {
            (id != graph.entry() && sources.len() == 1)
                .then(|| (id, *sources.first().expect("one predecessor")))
        });
        let Some((node, predecessor)) = collapse else {
            break;
        };

        for targets in successors.values_mut() {
            targets.remove(&node);
        }
        let node_successors = successors
            .remove(&node)
            .expect("collapse candidate is still present");
        let predecessor_successors = successors
            .get_mut(&predecessor)
            .expect("collapse predecessor is still present");
        predecessor_successors.extend(
            node_successors
                .into_iter()
                .filter(|&successor| successor != predecessor),
        );
    }

    successors.len() > 1
}

/// Hoists trap-free expressions whose inputs are invariant in a natural loop.
///
/// Only loops with a single, dedicated preheader are considered. Every input
/// that stays outside the loop must dominate that preheader and be scheduled
/// before its header. Nodes are otherwise kept in their original relative order.
pub fn hoist_loop_invariants(graph: &mut Graph) -> usize {
    let mut hoisted = 0;
    while let Some((header, candidates)) = next_hoist(graph) {
        let count = candidates.len();
        reorder_before(graph, header, &candidates);
        hoisted += count;
    }
    hoisted
}

struct SonControlFlow {
    blocks: BTreeMap<NodeId, NodeId>,
    successors: BTreeMap<NodeId, Vec<NodeId>>,
    roots: Vec<NodeId>,
}

impl SonControlFlow {
    fn new(graph: &Graph) -> Self {
        let mut blocks = BTreeMap::new();
        let mut roots = Vec::new();
        let mut current_block = None;
        for (id, node) in graph.iter() {
            if matches!(
                node.kind,
                NodeKind::Start | NodeKind::Projection(_) | NodeKind::Region
            ) {
                current_block = Some(id);
                if node.kind == NodeKind::Start {
                    roots.push(id);
                }
            }
            if let Some(block) = current_block {
                blocks.insert(id, block);
            }
        }

        let mut successors = BTreeMap::<NodeId, Vec<NodeId>>::new();
        for (id, node) in graph.iter() {
            if !matches!(node.kind, NodeKind::Projection(_) | NodeKind::Region) {
                continue;
            }
            let block = blocks[&id];
            for control in &node.control {
                if let Some(predecessor) = blocks.get(control) {
                    successors.entry(*predecessor).or_default().push(block);
                }
            }
        }
        Self {
            blocks,
            successors,
            roots,
        }
    }

    fn graph_for_root(&self, root: NodeId) -> ControlFlowGraph<()> {
        let mut reachable = BTreeSet::new();
        let mut pending = vec![root];
        while let Some(block) = pending.pop() {
            if reachable.insert(block) {
                pending.extend(self.successors.get(&block).into_iter().flatten().copied());
            }
        }

        ControlFlowGraph::new(
            BlockId(root.0),
            reachable.into_iter().map(|block| {
                let successors = self
                    .successors
                    .get(&block)
                    .into_iter()
                    .flatten()
                    .map(|successor| BlockId(successor.0))
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .collect();
                BasicBlock::new(BlockId(block.0), Vec::new(), successors)
            }),
        )
        .expect("SON control edges produce a valid CFG")
    }
}

fn next_hoist(graph: &Graph) -> Option<(NodeId, BTreeSet<NodeId>)> {
    let control = SonControlFlow::new(graph);
    let mut plans = Vec::new();
    for root in control.roots.iter().copied() {
        let cfg = control.graph_for_root(root);
        let analysis = analyze_loops(&cfg);
        for natural_loop in &analysis.loops {
            let header = NodeId(natural_loop.header.0);
            let Some(preheader) = dedicated_preheader(&cfg, natural_loop) else {
                continue;
            };
            let candidates = loop_invariant_candidates(
                graph,
                &control.blocks,
                &analysis,
                natural_loop,
                preheader,
                header,
            );
            let has_expression = candidates.iter().any(|id| {
                graph
                    .node(*id)
                    .is_some_and(|node| !matches!(node.kind, NodeKind::Const(_)))
            });
            if has_expression {
                plans.push((natural_loop.depth, header, candidates));
            }
        }
    }
    plans.sort_by(|left, right| right.0.cmp(&left.0).then_with(|| left.1.cmp(&right.1)));
    plans
        .into_iter()
        .next()
        .map(|(_, header, candidates)| (header, candidates))
}

fn dedicated_preheader(
    graph: &ControlFlowGraph<()>,
    natural_loop: &NaturalLoop,
) -> Option<BlockId> {
    let mut predecessors = graph
        .predecessors(natural_loop.header)?
        .iter()
        .copied()
        .filter(|predecessor| !natural_loop.contains(*predecessor));
    let preheader = predecessors.next()?;
    if predecessors.next().is_some() || graph.successors(preheader)? != [natural_loop.header] {
        return None;
    }
    Some(preheader)
}

fn loop_invariant_candidates(
    graph: &Graph,
    blocks: &BTreeMap<NodeId, NodeId>,
    analysis: &LoopAnalysis,
    natural_loop: &NaturalLoop,
    preheader: BlockId,
    header: NodeId,
) -> BTreeSet<NodeId> {
    let mut selected = BTreeSet::new();
    for id in graph.ids() {
        if selected.contains(&id) {
            continue;
        }
        let mut additional = BTreeSet::new();
        let mut visiting = BTreeSet::new();
        let context = InvariantContext {
            graph,
            blocks,
            analysis,
            natural_loop,
            preheader,
            header,
            selected: &selected,
        };
        if collect_loop_invariant(&context, id, &mut additional, &mut visiting) {
            selected.extend(additional);
        }
    }
    selected
}

struct InvariantContext<'a> {
    graph: &'a Graph,
    blocks: &'a BTreeMap<NodeId, NodeId>,
    analysis: &'a LoopAnalysis,
    natural_loop: &'a NaturalLoop,
    preheader: BlockId,
    header: NodeId,
    selected: &'a BTreeSet<NodeId>,
}

fn collect_loop_invariant(
    context: &InvariantContext,
    id: NodeId,
    additional: &mut BTreeSet<NodeId>,
    visiting: &mut BTreeSet<NodeId>,
) -> bool {
    if context.selected.contains(&id) || additional.contains(&id) {
        return true;
    }
    let Some(block) = context.blocks.get(&id).copied() else {
        return false;
    };
    if !context.natural_loop.contains(BlockId(block.0)) {
        return id < context.header
            && context
                .analysis
                .dominates(BlockId(block.0), context.preheader);
    }
    let Some(node) = context.graph.node(id) else {
        return false;
    };
    if !is_hoistable(node) || !visiting.insert(id) {
        return false;
    }
    for input in &node.data {
        if !collect_loop_invariant(context, *input, additional, visiting) {
            visiting.remove(&id);
            return false;
        }
    }
    visiting.remove(&id);
    additional.insert(id);
    true
}

fn is_hoistable(node: &crate::son::Node) -> bool {
    if !node.control.is_empty() || !node.memory.is_empty() {
        return false;
    }
    match node.kind {
        NodeKind::Const(_) => node.data.is_empty(),
        NodeKind::Unary(_) => node.data.len() == 1,
        NodeKind::Binary(
            BinaryOp::Add
            | BinaryOp::Sub
            | BinaryOp::Mul
            | BinaryOp::And
            | BinaryOp::Or
            | BinaryOp::Xor
            | BinaryOp::Shl
            | BinaryOp::Shr
            | BinaryOp::LShr,
        )
        | NodeKind::Compare(_) => node.data.len() == 2,
        _ => false,
    }
}

fn reorder_before(graph: &mut Graph, header: NodeId, candidates: &BTreeSet<NodeId>) {
    let hoisted = graph
        .ids()
        .filter(|id| candidates.contains(id))
        .collect::<Vec<_>>();
    let mut ordered = graph
        .ids()
        .filter(|id| !candidates.contains(id))
        .collect::<Vec<_>>();
    let insertion = ordered
        .iter()
        .position(|id| *id == header)
        .expect("loop header remains in the graph");
    ordered.splice(insertion..insertion, hoisted);

    let mapping = ordered
        .iter()
        .enumerate()
        .map(|(index, id)| (*id, NodeId(index as u32)))
        .collect::<BTreeMap<_, _>>();
    let mut reordered = Graph::new();
    for old_id in ordered {
        let mut node = graph.node(old_id).expect("ordered node exists").clone();
        for channel in [
            InputChannel::Control,
            InputChannel::Data,
            InputChannel::Memory,
        ] {
            for input in node.inputs_mut(channel) {
                *input = mapping[input];
            }
        }
        reordered.push(node);
    }
    reordered.functions = graph.functions.clone();
    for function in &mut reordered.functions {
        function.start = mapping[&function.start];
        function.return_node = mapping[&function.return_node];
    }
    *graph = reordered;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cfg::BasicBlock;

    fn id(raw: u32) -> BlockId {
        BlockId(raw)
    }

    fn block(raw: u32, successors: &[u32]) -> BasicBlock<()> {
        BasicBlock::new(
            id(raw),
            Vec::new(),
            successors.iter().copied().map(id).collect(),
        )
    }

    fn graph(entry: u32, blocks: Vec<BasicBlock<()>>) -> ControlFlowGraph<()> {
        ControlFlowGraph::new(id(entry), blocks).unwrap()
    }

    fn blocks(values: &[u32]) -> BTreeSet<BlockId> {
        values.iter().copied().map(id).collect()
    }

    #[test]
    fn single_loop_has_dominance_back_edge_and_natural_body() {
        let graph = graph(
            0,
            vec![
                block(0, &[1]),
                block(1, &[2, 3]),
                block(2, &[1]),
                block(3, &[]),
            ],
        );
        let analysis = graph.loop_analysis();

        assert_eq!(analysis.dominators(id(0)), Some(&blocks(&[0])));
        assert_eq!(analysis.dominators(id(1)), Some(&blocks(&[0, 1])));
        assert_eq!(analysis.dominators(id(2)), Some(&blocks(&[0, 1, 2])));
        assert_eq!(analysis.dominators(id(3)), Some(&blocks(&[0, 1, 3])));
        assert!(analysis.dominates(id(1), id(2)));
        assert!(!analysis.dominates(id(2), id(1)));
        assert!(!analysis.dominates(id(99), id(1)));
        assert_eq!(analysis.immediate_dominator(id(0)), None);
        assert_eq!(analysis.immediate_dominator(id(2)), Some(id(1)));
        assert_eq!(
            analysis.dominator_children(id(1)),
            Some(&[id(2), id(3)][..])
        );
        assert_eq!(
            analysis.back_edges,
            vec![BackEdge {
                from: id(2),
                to: id(1)
            }]
        );

        let natural_loop = analysis.loop_for_header(id(1)).unwrap();
        assert_eq!(natural_loop.blocks, blocks(&[1, 2]));
        assert_eq!(natural_loop.parent, None);
        assert_eq!(natural_loop.depth, 0);
        assert!(!natural_loop.contains(id(0)));
        assert!(!natural_loop.contains(id(3)));
        assert!(!analysis.is_irreducible());
    }

    #[test]
    fn nested_loops_have_immediate_parents_and_depths() {
        let graph = graph(
            0,
            vec![
                block(0, &[1]),
                block(1, &[2, 6]),
                block(2, &[3, 5]),
                block(3, &[4]),
                block(4, &[2]),
                block(5, &[1]),
                block(6, &[]),
            ],
        );
        let analysis = analyze_loops(&graph);

        assert_eq!(analysis.loops.len(), 2);
        let outer = analysis.loop_for_header(id(1)).unwrap();
        let inner = analysis.loop_for_header(id(2)).unwrap();
        assert_eq!(outer.blocks, blocks(&[1, 2, 3, 4, 5]));
        assert_eq!(outer.parent, None);
        assert_eq!(outer.depth, 0);
        assert_eq!(inner.blocks, blocks(&[2, 3, 4]));
        assert_eq!(inner.parent, Some(id(1)));
        assert_eq!(inner.depth, 1);
        assert_eq!(analysis.innermost_loop(id(3)), Some(inner));
        assert_eq!(analysis.innermost_loop(id(5)), Some(outer));
        assert_eq!(analysis.innermost_loop(id(6)), None);
        assert!(!analysis.irreducible);
    }

    #[test]
    fn break_and_continue_edges_share_one_loop_and_exclude_exit() {
        let graph = graph(
            0,
            vec![
                block(0, &[1]),
                block(1, &[2, 3]),
                block(2, &[1]),
                block(3, &[4, 5]),
                block(4, &[1]),
                block(5, &[]),
            ],
        );
        let analysis = graph.loop_analysis();

        assert_eq!(
            analysis.back_edges,
            vec![
                BackEdge {
                    from: id(2),
                    to: id(1)
                },
                BackEdge {
                    from: id(4),
                    to: id(1)
                },
            ]
        );
        assert_eq!(analysis.loops.len(), 1);
        let natural_loop = &analysis.loops[0];
        assert_eq!(natural_loop.blocks, blocks(&[1, 2, 3, 4]));
        assert_eq!(natural_loop.back_edges, analysis.back_edges);
        assert!(!natural_loop.contains(id(5)));
        assert!(!analysis.irreducible);
    }

    #[test]
    fn multi_entry_cycle_is_explicitly_irreducible() {
        let graph = graph(
            0,
            vec![
                block(0, &[1, 2]),
                block(1, &[2]),
                block(2, &[1, 3]),
                block(3, &[]),
            ],
        );
        let analysis = graph.loop_analysis();

        assert!(analysis.is_irreducible());
        assert!(analysis.back_edges.is_empty());
        assert!(analysis.loops.is_empty());
        assert_eq!(analysis.immediate_dominator(id(1)), Some(id(0)));
        assert_eq!(analysis.immediate_dominator(id(2)), Some(id(0)));
        assert!(analysis.unreachable_blocks.is_empty());
    }

    #[test]
    fn unreachable_blocks_and_cycles_do_not_pollute_analysis() {
        let graph = graph(
            0,
            vec![
                block(4, &[4]),
                block(2, &[3]),
                block(0, &[1]),
                block(3, &[2]),
                block(1, &[]),
            ],
        );
        let analysis = graph.loop_analysis();

        assert_eq!(analysis.unreachable_blocks, blocks(&[2, 3, 4]));
        assert_eq!(analysis.dominators.len(), 2);
        assert_eq!(analysis.dominators(id(2)), None);
        assert!(!analysis.dominates(id(2), id(3)));
        assert!(analysis.back_edges.is_empty());
        assert!(analysis.loops.is_empty());
        assert!(!analysis.irreducible);
    }

    #[test]
    fn output_is_deterministic_across_input_and_successor_order() {
        let forward = graph(
            0,
            vec![
                block(0, &[1, 2]),
                block(1, &[3]),
                block(2, &[3]),
                block(3, &[4, 5]),
                block(4, &[3]),
                block(5, &[]),
            ],
        );
        let reversed = graph(
            0,
            vec![
                block(5, &[]),
                block(4, &[3]),
                block(3, &[5, 4]),
                block(2, &[3]),
                block(1, &[3]),
                block(0, &[2, 1]),
            ],
        );

        let expected = analyze_loops(&forward);
        assert_eq!(analyze_loops(&reversed), expected);
        assert_eq!(analyze_loops(&forward), expected);
    }
}
