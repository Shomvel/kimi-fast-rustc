//! Target-independent control-flow graphs and iterative liveness analysis.
//!
//! Blocks and instructions retain their input order. Reverse postorder contains
//! only blocks reachable from the entry, while liveness is computed for every
//! block, including unreachable blocks and unreachable cycles. This module does
//! not schedule or reorder instructions.

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct BlockId(pub u32);

impl fmt::Display for BlockId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "bb{}", self.0)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Instruction<V> {
    pub defs: Vec<V>,
    pub uses: Vec<V>,
}

impl<V> Instruction<V> {
    pub fn new(defs: Vec<V>, uses: Vec<V>) -> Self {
        Self { defs, uses }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BasicBlock<V> {
    pub id: BlockId,
    pub instructions: Vec<Instruction<V>>,
    pub successors: Vec<BlockId>,
}

impl<V> BasicBlock<V> {
    pub fn new(id: BlockId, instructions: Vec<Instruction<V>>, successors: Vec<BlockId>) -> Self {
        Self {
            id,
            instructions,
            successors,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CfgError {
    EmptyGraph,
    MissingEntry(BlockId),
    DuplicateBlock(BlockId),
    UnknownSuccessor { block: BlockId, successor: BlockId },
    DuplicateSuccessor { block: BlockId, successor: BlockId },
}

impl fmt::Display for CfgError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyGraph => formatter.write_str("control-flow graph has no blocks"),
            Self::MissingEntry(entry) => write!(formatter, "entry block {entry} does not exist"),
            Self::DuplicateBlock(block) => write!(formatter, "duplicate block {block}"),
            Self::UnknownSuccessor { block, successor } => {
                write!(formatter, "block {block} has unknown successor {successor}")
            }
            Self::DuplicateSuccessor { block, successor } => {
                write!(formatter, "block {block} repeats successor {successor}")
            }
        }
    }
}

impl Error for CfgError {}

#[derive(Clone, Debug)]
pub struct ControlFlowGraph<V> {
    entry: BlockId,
    blocks: BTreeMap<BlockId, BasicBlock<V>>,
    predecessors: BTreeMap<BlockId, Vec<BlockId>>,
    reverse_postorder: Vec<BlockId>,
}

/// Short name for [`ControlFlowGraph`].
pub type Cfg<V> = ControlFlowGraph<V>;

impl<V> ControlFlowGraph<V> {
    /// Builds and validates a graph. Predecessors are derived from successors.
    /// Unreachable blocks are valid and participate in validation and liveness.
    pub fn new(
        entry: BlockId,
        blocks: impl IntoIterator<Item = BasicBlock<V>>,
    ) -> Result<Self, CfgError> {
        let blocks = blocks.into_iter().collect::<Vec<_>>();
        validate_blocks(entry, &blocks)?;

        let blocks = blocks
            .into_iter()
            .map(|block| (block.id, block))
            .collect::<BTreeMap<_, _>>();
        let mut predecessors = blocks
            .keys()
            .map(|&id| (id, Vec::new()))
            .collect::<BTreeMap<_, _>>();
        for block in blocks.values() {
            for &successor in &block.successors {
                predecessors
                    .get_mut(&successor)
                    .expect("validation checked successor")
                    .push(block.id);
            }
        }

        let reverse_postorder = compute_reverse_postorder(entry, &blocks);
        Ok(Self {
            entry,
            blocks,
            predecessors,
            reverse_postorder,
        })
    }

    pub fn entry(&self) -> BlockId {
        self.entry
    }

    pub fn len(&self) -> usize {
        self.blocks.len()
    }

    pub fn is_empty(&self) -> bool {
        self.blocks.is_empty()
    }

    /// Iterates over blocks in ascending `BlockId` order without changing their
    /// instruction order.
    pub fn iter(&self) -> impl Iterator<Item = &BasicBlock<V>> {
        self.blocks.values()
    }

    pub fn block(&self, id: BlockId) -> Option<&BasicBlock<V>> {
        self.blocks.get(&id)
    }

    pub fn successors(&self, id: BlockId) -> Option<&[BlockId]> {
        self.blocks
            .get(&id)
            .map(|block| block.successors.as_slice())
    }

    pub fn predecessors(&self, id: BlockId) -> Option<&[BlockId]> {
        self.predecessors.get(&id).map(Vec::as_slice)
    }

    pub fn is_reachable(&self, id: BlockId) -> bool {
        self.reverse_postorder.contains(&id)
    }

    /// Returns the deterministic DFS reverse postorder from the entry.
    /// Successors are visited in their declared order; unreachable blocks are
    /// omitted.
    pub fn reverse_postorder(&self) -> &[BlockId] {
        &self.reverse_postorder
    }

    /// Computes iterative liveness using the standard equations:
    /// `out[b] = union(in[s])` and `in[b] = use[b] union (out[b] - def[b])`.
    /// Analysis reaches a fixed point even when back edges or self loops exist.
    pub fn liveness(&self) -> Liveness<V>
    where
        V: Clone + Ord,
    {
        compute_liveness(self)
    }
}

/// Validates block IDs, the entry, and all edges without constructing a graph.
pub fn validate_blocks<V>(entry: BlockId, blocks: &[BasicBlock<V>]) -> Result<(), CfgError> {
    if blocks.is_empty() {
        return Err(CfgError::EmptyGraph);
    }

    let mut ids = BTreeSet::new();
    for block in blocks {
        if !ids.insert(block.id) {
            return Err(CfgError::DuplicateBlock(block.id));
        }
    }
    if !ids.contains(&entry) {
        return Err(CfgError::MissingEntry(entry));
    }

    for block in blocks {
        let mut successors = BTreeSet::new();
        for &successor in &block.successors {
            if !ids.contains(&successor) {
                return Err(CfgError::UnknownSuccessor {
                    block: block.id,
                    successor,
                });
            }
            if !successors.insert(successor) {
                return Err(CfgError::DuplicateSuccessor {
                    block: block.id,
                    successor,
                });
            }
        }
    }
    Ok(())
}

fn compute_reverse_postorder<V>(
    entry: BlockId,
    blocks: &BTreeMap<BlockId, BasicBlock<V>>,
) -> Vec<BlockId> {
    let mut visited = BTreeSet::from([entry]);
    let mut stack = vec![(entry, 0usize)];
    let mut postorder = Vec::with_capacity(blocks.len());

    while let Some((id, next_successor)) = stack.last_mut() {
        let block = blocks.get(id).expect("entry and successors are validated");
        if *next_successor < block.successors.len() {
            let successor = block.successors[*next_successor];
            *next_successor += 1;
            if visited.insert(successor) {
                stack.push((successor, 0));
            }
        } else {
            postorder.push(*id);
            stack.pop();
        }
    }

    postorder.reverse();
    postorder
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Liveness<V> {
    pub live_in: BTreeMap<BlockId, BTreeSet<V>>,
    pub live_out: BTreeMap<BlockId, BTreeSet<V>>,
}

impl<V> Liveness<V> {
    pub fn live_in(&self, block: BlockId) -> Option<&BTreeSet<V>> {
        self.live_in.get(&block)
    }

    pub fn live_out(&self, block: BlockId) -> Option<&BTreeSet<V>> {
        self.live_out.get(&block)
    }
}

/// Computes liveness for all blocks. Unknown block queries return `None` from
/// the accessors on [`Liveness`].
pub fn compute_liveness<V>(graph: &ControlFlowGraph<V>) -> Liveness<V>
where
    V: Clone + Ord,
{
    let empty_sets = || {
        graph
            .blocks
            .keys()
            .map(|&id| (id, BTreeSet::new()))
            .collect::<BTreeMap<_, _>>()
    };
    let mut live_in = empty_sets();
    let mut live_out = empty_sets();

    let mut analysis_order = graph.reverse_postorder.clone();
    analysis_order.extend(
        graph
            .blocks
            .keys()
            .filter(|id| !graph.reverse_postorder.contains(id)),
    );
    analysis_order.reverse();

    loop {
        let mut changed = false;
        for &id in &analysis_order {
            let block = graph.blocks.get(&id).expect("analysis order has valid IDs");
            let mut new_out = BTreeSet::new();
            for successor in &block.successors {
                let successor_in = live_in
                    .get(successor)
                    .expect("validation checked successor");
                new_out.extend(successor_in.iter().cloned());
            }

            let mut new_in = new_out.clone();
            for instruction in block.instructions.iter().rev() {
                for def in &instruction.defs {
                    new_in.remove(def);
                }
                for used in &instruction.uses {
                    new_in.insert(used.clone());
                }
            }

            let old_out = live_out.get_mut(&id).expect("all blocks initialized");
            if *old_out != new_out {
                *old_out = new_out;
                changed = true;
            }
            let old_in = live_in.get_mut(&id).expect("all blocks initialized");
            if *old_in != new_in {
                *old_in = new_in;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }

    Liveness { live_in, live_out }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(raw: u32) -> BlockId {
        BlockId(raw)
    }

    fn instruction(defs: &[&'static str], uses: &[&'static str]) -> Instruction<&'static str> {
        Instruction::new(defs.to_vec(), uses.to_vec())
    }

    fn block(
        raw: u32,
        instructions: Vec<Instruction<&'static str>>,
        successors: &[u32],
    ) -> BasicBlock<&'static str> {
        BasicBlock::new(
            id(raw),
            instructions,
            successors.iter().copied().map(id).collect(),
        )
    }

    fn set(values: &[&'static str]) -> BTreeSet<&'static str> {
        values.iter().copied().collect()
    }

    #[test]
    fn branch_liveness_propagates_through_join_and_predecessors_are_derived() {
        let graph = ControlFlowGraph::new(
            id(0),
            vec![
                block(0, vec![instruction(&["x", "y"], &[])], &[1, 2]),
                block(1, vec![instruction(&[], &["x"])], &[3]),
                block(2, vec![instruction(&[], &["y"])], &[3]),
                block(3, vec![instruction(&[], &["z"])], &[]),
            ],
        )
        .unwrap();

        assert_eq!(graph.predecessors(id(0)), Some(&[][..]));
        assert_eq!(graph.predecessors(id(3)), Some(&[id(1), id(2)][..]));
        assert_eq!(graph.successors(id(0)), Some(&[id(1), id(2)][..]));
        assert_eq!(graph.reverse_postorder(), &[id(0), id(2), id(1), id(3)]);

        let live = graph.liveness();
        assert_eq!(live.live_out(id(0)), Some(&set(&["x", "y", "z"])));
        assert_eq!(live.live_in(id(0)), Some(&set(&["z"])));
        assert_eq!(live.live_out(id(1)), Some(&set(&["z"])));
        assert_eq!(live.live_out(id(2)), Some(&set(&["z"])));
        assert_eq!(live.live_in(id(3)), Some(&set(&["z"])));
        assert_eq!(live.live_out(id(3)), Some(&BTreeSet::new()));
    }

    #[test]
    fn loop_liveness_iterates_to_a_fixed_point() {
        let graph = ControlFlowGraph::new(
            id(0),
            vec![
                block(0, vec![instruction(&["i", "sum"], &[])], &[1]),
                block(1, vec![instruction(&[], &["i"])], &[2, 3]),
                block(2, vec![instruction(&["i", "sum"], &["i", "sum"])], &[1]),
                block(3, vec![instruction(&[], &["sum"])], &[]),
            ],
        )
        .unwrap();

        assert_eq!(graph.predecessors(id(1)), Some(&[id(0), id(2)][..]));
        assert_eq!(graph.reverse_postorder(), &[id(0), id(1), id(3), id(2)]);
        let live = graph.liveness();
        assert_eq!(live.live_in(id(0)), Some(&BTreeSet::new()));
        assert_eq!(live.live_out(id(0)), Some(&set(&["i", "sum"])));
        assert_eq!(live.live_in(id(1)), Some(&set(&["i", "sum"])));
        assert_eq!(live.live_out(id(1)), Some(&set(&["i", "sum"])));
        assert_eq!(live.live_in(id(2)), Some(&set(&["i", "sum"])));
        assert_eq!(live.live_out(id(2)), Some(&set(&["i", "sum"])));
        assert_eq!(live.live_in(id(3)), Some(&set(&["sum"])));
    }

    #[test]
    fn unreachable_blocks_and_cycles_are_analyzed_but_not_in_rpo() {
        let graph = ControlFlowGraph::new(
            id(0),
            vec![
                block(3, vec![instruction(&["loop"], &["loop"])], &[3]),
                block(1, vec![instruction(&[], &["exit"])], &[]),
                block(0, Vec::new(), &[1]),
                block(2, vec![instruction(&[], &["ghost"])], &[1]),
            ],
        )
        .unwrap();

        assert_eq!(graph.reverse_postorder(), &[id(0), id(1)]);
        assert!(graph.is_reachable(id(0)));
        assert!(!graph.is_reachable(id(2)));
        assert!(!graph.is_reachable(id(3)));
        assert_eq!(graph.predecessors(id(1)), Some(&[id(0), id(2)][..]));
        assert_eq!(graph.predecessors(id(3)), Some(&[id(3)][..]));

        let live = graph.liveness();
        assert_eq!(live.live_in(id(0)), Some(&set(&["exit"])));
        assert_eq!(live.live_in(id(2)), Some(&set(&["exit", "ghost"])));
        assert_eq!(live.live_out(id(2)), Some(&set(&["exit"])));
        assert_eq!(live.live_in(id(3)), Some(&set(&["loop"])));
        assert_eq!(live.live_out(id(3)), Some(&set(&["loop"])));
        assert_eq!(live.live_in(id(99)), None);
    }

    #[test]
    fn validation_reports_precise_errors_even_for_unreachable_blocks() {
        assert_eq!(
            ControlFlowGraph::<&str>::new(id(0), Vec::new()).unwrap_err(),
            CfgError::EmptyGraph
        );
        assert_eq!(
            ControlFlowGraph::new(id(9), vec![block(0, Vec::new(), &[])]).unwrap_err(),
            CfgError::MissingEntry(id(9))
        );
        assert_eq!(
            ControlFlowGraph::new(
                id(0),
                vec![block(0, Vec::new(), &[]), block(0, Vec::new(), &[])],
            )
            .unwrap_err(),
            CfgError::DuplicateBlock(id(0))
        );
        assert_eq!(
            ControlFlowGraph::new(
                id(0),
                vec![block(0, Vec::new(), &[]), block(1, Vec::new(), &[99])],
            )
            .unwrap_err(),
            CfgError::UnknownSuccessor {
                block: id(1),
                successor: id(99),
            }
        );
        assert_eq!(
            ControlFlowGraph::new(
                id(0),
                vec![block(0, Vec::new(), &[1, 1]), block(1, Vec::new(), &[])],
            )
            .unwrap_err(),
            CfgError::DuplicateSuccessor {
                block: id(0),
                successor: id(1),
            }
        );
    }

    #[test]
    fn instruction_order_is_preserved_and_defs_kill_before_uses_gen() {
        let instructions = vec![
            instruction(&["x"], &["a"]),
            instruction(&["y"], &["x"]),
            instruction(&["x"], &["y"]),
        ];
        let graph =
            ControlFlowGraph::new(id(0), vec![block(0, instructions.clone(), &[])]).unwrap();

        assert_eq!(graph.block(id(0)).unwrap().instructions, instructions);
        let live = graph.liveness();
        assert_eq!(live.live_in(id(0)), Some(&set(&["a"])));
        assert_eq!(live.live_out(id(0)), Some(&BTreeSet::new()));
    }

    #[test]
    fn construction_and_liveness_are_deterministic_across_block_input_order() {
        let blocks = vec![
            block(0, vec![instruction(&["x"], &["seed"])], &[1, 2]),
            block(1, vec![instruction(&[], &["x"])], &[3]),
            block(2, vec![instruction(&["x"], &["x"])], &[1]),
            block(3, vec![instruction(&[], &["x"])], &[]),
        ];
        let forward = ControlFlowGraph::new(id(0), blocks.clone()).unwrap();
        let reverse = ControlFlowGraph::new(id(0), blocks.into_iter().rev()).unwrap();

        assert_eq!(forward.reverse_postorder(), reverse.reverse_postorder());
        assert_eq!(forward.predecessors, reverse.predecessors);
        assert_eq!(forward.liveness(), reverse.liveness());
        assert_eq!(forward.liveness(), forward.liveness());
    }
}
