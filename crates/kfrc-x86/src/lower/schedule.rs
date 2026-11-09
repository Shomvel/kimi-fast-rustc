//! Control scheduling and global code motion of pure graph nodes.
use super::*;

pub(super) struct Block {
    pub(super) head: NodeId,
    /// Fixed nodes in execution order; the last one ends the block.
    pub(super) chain: Vec<NodeId>,
    pub(super) successors: Vec<usize>,
}

pub(super) struct Schedule {
    pub(super) blocks: Vec<Block>,
    /// Blocks in emission order (reverse postorder).
    pub(super) order: Vec<usize>,
    pub(super) block_of_head: HashMap<NodeId, usize>,
    /// Pure nodes placed in each block.
    pub(super) placed: Vec<Vec<NodeId>>,
    pub(super) region_of_jump: HashMap<NodeId, (NodeId, usize)>,
    pub(super) phis_of_region: HashMap<NodeId, Vec<NodeId>>,
    pub(super) projections: HashMap<(NodeId, u32), NodeId>,
}

impl Schedule {
    pub(super) fn new(graph: &FunctionGraph) -> Result<Self, LowerError> {
        let mut control_users: HashMap<NodeId, Vec<NodeId>> = HashMap::new();
        let mut region_of_jump = HashMap::new();
        let mut phis_of_region: HashMap<NodeId, Vec<NodeId>> = HashMap::new();
        let mut projections = HashMap::new();
        for node in graph.ids() {
            let control = graph.control(node);
            if !control.is_none() {
                control_users.entry(control).or_default().push(node);
            }
            match graph.op(node) {
                Opcode::Region => {
                    for (slot, jump) in graph.inputs(node).iter().enumerate() {
                        region_of_jump.insert(*jump, (node, slot));
                    }
                }
                Opcode::Phi => phis_of_region
                    .entry(graph.input(node, 0))
                    .or_default()
                    .push(node),
                Opcode::Projection => {
                    projections.insert((graph.input(node, 0), graph.payload(node)), node);
                }
                _ => {}
            }
        }
        let entry = *projections
            .get(&(graph.start, 0))
            .ok_or_else(|| LowerError::Malformed("start has no control projection".to_owned()))?;

        // Discover blocks from the entry by following control.
        let mut blocks: Vec<Block> = Vec::new();
        let mut block_of_head: HashMap<NodeId, usize> = HashMap::new();
        let mut worklist = vec![entry];
        block_of_head.insert(entry, 0);
        blocks.push(Block {
            head: entry,
            chain: Vec::new(),
            successors: Vec::new(),
        });
        let mut placement: HashMap<NodeId, usize> = HashMap::new();
        while let Some(head) = worklist.pop() {
            let index = block_of_head[&head];
            placement.insert(head, index);
            let mut control = head;
            let mut chain = Vec::new();
            let mut successors_heads = Vec::new();
            loop {
                let users = control_users.get(&control).cloned().unwrap_or_default();
                chain.extend(anchored_order(graph, control, &users)?);
                let next = users
                    .iter()
                    .copied()
                    .find(|user| {
                        matches!(
                            graph.op(*user),
                            Opcode::Call
                                | Opcode::CallIndirect
                                | Opcode::ThreadLocalAddress
                                | Opcode::Opaque
                                | Opcode::SlotLive
                                | Opcode::SlotDead
                                | Opcode::Load
                                | Opcode::Store
                                | Opcode::If
                                | Opcode::Jump
                                | Opcode::Return
                                | Opcode::Unreachable
                        )
                    })
                    .ok_or_else(|| {
                        LowerError::Malformed(format!("control {control} has no successor"))
                    })?;
                chain.push(next);
                match graph.op(next) {
                    Opcode::Call
                    | Opcode::CallIndirect
                    | Opcode::ThreadLocalAddress
                    | Opcode::Opaque
                    | Opcode::SlotLive
                    | Opcode::SlotDead
                    | Opcode::Load
                    | Opcode::Store => {
                        control = *projections.get(&(next, 0)).ok_or_else(|| {
                            LowerError::Malformed(format!("{next} lacks a control projection"))
                        })?;
                        placement.insert(control, index);
                    }
                    Opcode::If => {
                        for branch in [0, 1] {
                            let projection =
                                *projections.get(&(next, branch)).ok_or_else(|| {
                                    LowerError::Malformed(format!(
                                        "{next} lacks projection {branch}"
                                    ))
                                })?;
                            successors_heads.push(projection);
                        }
                        break;
                    }
                    Opcode::Jump => {
                        let (region, _) = *region_of_jump.get(&next).ok_or_else(|| {
                            LowerError::Malformed(format!("{next} jumps nowhere"))
                        })?;
                        successors_heads.push(region);
                        break;
                    }
                    _ => break,
                }
            }
            let mut successors = Vec::new();
            for successor in successors_heads {
                let successor_index = match block_of_head.get(&successor) {
                    Some(existing) => *existing,
                    None => {
                        let new_index = blocks.len();
                        blocks.push(Block {
                            head: successor,
                            chain: Vec::new(),
                            successors: Vec::new(),
                        });
                        block_of_head.insert(successor, new_index);
                        worklist.push(successor);
                        new_index
                    }
                };
                successors.push(successor_index);
            }
            for node in &chain {
                placement.insert(*node, index);
            }
            blocks[index].chain = chain;
            blocks[index].successors = successors;
        }

        let order = reverse_postorder(&blocks);
        let rank: HashMap<usize, usize> = order
            .iter()
            .enumerate()
            .map(|(rank, block)| (*block, rank))
            .collect();
        let idom = dominators(&blocks, &order, &rank);
        let depth = dominator_depths(&order, &idom);

        // Place value nodes at their earliest block, then move each to the
        // latest block that keeps it out of loops.
        let mut placed = vec![Vec::new(); blocks.len()];
        let ids: Vec<NodeId> = graph.ids().collect();
        for node in &ids {
            place(graph, *node, &mut placement, &mut placed, &depth)?;
        }
        let placed = schedule_late(graph, &blocks, &order, &rank, &idom, &placement, placed);
        Ok(Self {
            blocks,
            order,
            block_of_head,
            placed,
            region_of_jump,
            phis_of_region,
            projections,
        })
    }
}

/// Arena order is not execution order: lowering/inlining can add a division that
/// feeds an older division at the same anchor. Include dependencies through pure
/// expressions, without moving fixed values across their verified control.
fn anchored_order(
    graph: &FunctionGraph,
    control: NodeId,
    users: &[NodeId],
) -> Result<Vec<NodeId>, LowerError> {
    use kfrc_son::analysis::cfg::is_anchored_value;

    let mut output = Vec::new();
    let mut finished = HashSet::new();
    let mut active = HashSet::new();
    for &root in users
        .iter()
        .filter(|&&node| is_anchored_value(graph.op(node)))
    {
        let mut work = vec![(root, false)];
        while let Some((node, exiting)) = work.pop() {
            if exiting {
                active.remove(&node);
                finished.insert(node);
                if is_anchored_value(graph.op(node)) {
                    output.push(node);
                }
                continue;
            }
            if finished.contains(&node) {
                continue;
            }
            let opcode = graph.op(node);
            if matches!(opcode, Opcode::Phi | Opcode::Projection)
                || (opcode.is_fixed()
                    && (!is_anchored_value(opcode) || graph.control(node) != control))
            {
                continue;
            }
            if !active.insert(node) {
                return Err(LowerError::Malformed(format!(
                    "cyclic anchored-value dependencies at {node}"
                )));
            }
            work.push((node, true));
            for &input in graph.inputs(node).iter().rev() {
                work.push((input, false));
            }
        }
    }
    Ok(output)
}

fn place(
    graph: &FunctionGraph,
    node: NodeId,
    placement: &mut HashMap<NodeId, usize>,
    placed: &mut [Vec<NodeId>],
    depth: &HashMap<usize, usize>,
) -> Result<usize, LowerError> {
    if let Some(block) = placement.get(&node) {
        return Ok(*block);
    }
    let block = match graph.op(node) {
        Opcode::Start | Opcode::Const | Opcode::Undefined => 0,
        Opcode::StackSlot | Opcode::SymbolAddress => {
            placed[0].push(node);
            0
        }
        Opcode::Phi => {
            let region = graph.input(node, 0);
            *placement.get(&region).ok_or_else(|| {
                LowerError::Malformed(format!("phi {node} of an unreachable region"))
            })?
        }
        Opcode::Projection => {
            let source = graph.input(node, 0);
            match graph.op(source) {
                Opcode::Start => 0,
                _ => *placement.get(&source).ok_or_else(|| {
                    LowerError::Malformed(format!("projection {node} of unscheduled {source}"))
                })?,
            }
        }
        Opcode::Region
        | Opcode::Jump
        | Opcode::If
        | Opcode::Call
        | Opcode::CallIndirect
        | Opcode::ThreadLocalAddress
        | Opcode::Opaque
        | Opcode::SlotLive
        | Opcode::SlotDead
        | Opcode::Load
        | Opcode::Store
        | Opcode::Return
        | Opcode::Unreachable => {
            return Err(LowerError::Malformed(format!(
                "control node {node} is unreachable"
            )));
        }
        _ => {
            let mut deepest = 0;
            for input in graph.inputs(node).to_vec() {
                let input_block = place(graph, input, placement, placed, depth)?;
                if depth.get(&input_block).copied().unwrap_or(0)
                    > depth.get(&deepest).copied().unwrap_or(0)
                {
                    deepest = input_block;
                }
            }
            placed[deepest].push(node);
            deepest
        }
    };
    placement.insert(node, block);
    Ok(block)
}

fn reverse_postorder(blocks: &[Block]) -> Vec<usize> {
    let mut visited = vec![false; blocks.len()];
    let mut postorder = Vec::new();
    let mut stack = vec![(0usize, 0usize)];
    visited[0] = true;
    while let Some((block, next)) = stack.pop() {
        if next < blocks[block].successors.len() {
            stack.push((block, next + 1));
            let successor = blocks[block].successors[next];
            if !visited[successor] {
                visited[successor] = true;
                stack.push((successor, 0));
            }
        } else {
            postorder.push(block);
        }
    }
    postorder.reverse();
    postorder
}

fn dominators(blocks: &[Block], order: &[usize], rank: &HashMap<usize, usize>) -> Vec<usize> {
    let mut predecessors = vec![Vec::new(); order.len()];
    for (block_rank, block) in order.iter().enumerate() {
        for successor in &blocks[*block].successors {
            predecessors[rank[successor]].push(block_rank);
        }
    }
    let mut idom = vec![usize::MAX; order.len()];
    idom[0] = 0;
    let mut changed = true;
    while changed {
        changed = false;
        for block_rank in 1..order.len() {
            let mut candidate = usize::MAX;
            for &predecessor in &predecessors[block_rank] {
                if idom[predecessor] == usize::MAX {
                    continue;
                }
                candidate = if candidate == usize::MAX {
                    predecessor
                } else {
                    let (mut a, mut b) = (predecessor, candidate);
                    while a != b {
                        while a > b {
                            a = idom[a];
                        }
                        while b > a {
                            b = idom[b];
                        }
                    }
                    a
                };
            }
            if candidate != idom[block_rank] {
                idom[block_rank] = candidate;
                changed = true;
            }
        }
    }
    idom
}

/// Dominator-tree depth per block id.
fn dominator_depths(order: &[usize], idom: &[usize]) -> HashMap<usize, usize> {
    let mut depth_by_rank = vec![0usize; order.len()];
    for rank in 1..order.len() {
        depth_by_rank[rank] = depth_by_rank[idom[rank]] + 1;
    }
    order
        .iter()
        .enumerate()
        .map(|(rank, block)| (*block, depth_by_rank[rank]))
        .collect()
}

/// Click's global code motion (PLDI 1995), late half. Every pure value moves
/// from its earliest block to the block on the dominator path between that
/// block and the LCA of its uses with the smallest loop depth, preferring the
/// latest. Short live ranges keep values in registers; loop-invariant values
/// stay hoisted. A value without uses is not placed and so never computed.
fn schedule_late(
    graph: &FunctionGraph,
    blocks: &[Block],
    order: &[usize],
    rank: &HashMap<usize, usize>,
    idom: &[usize],
    placement: &HashMap<NodeId, usize>,
    early: Vec<Vec<NodeId>>,
) -> Vec<Vec<NodeId>> {
    let mut early_rank = HashMap::new();
    for (block, nodes) in early.iter().enumerate() {
        for &node in nodes {
            early_rank.insert(node, rank[&block]);
        }
    }
    let mut users: HashMap<NodeId, Vec<(NodeId, usize)>> = HashMap::new();
    let mut pinned = HashSet::new();
    for node in graph.ids() {
        for (slot, &input) in graph.inputs(node).iter().enumerate() {
            let input = graph.resolve(input);
            users.entry(input).or_default().push((node, slot));
            // Tuple results keep their projections in the defining block.
            if graph.op(node) == Opcode::Projection {
                pinned.insert(input);
            }
        }
        let control = graph.control(node);
        if !control.is_none() {
            pinned.insert(graph.resolve(control));
        }
    }
    let late = Late {
        graph,
        rank,
        idom,
        placement,
        early_rank: &early_rank,
        users: &users,
        pinned: &pinned,
        loop_depth: loop_depths(blocks, order, rank, idom),
    };
    let mut decided = HashMap::new();
    let mut placed = vec![Vec::new(); blocks.len()];
    for node in early.iter().flatten() {
        if let Some(block_rank) = late.block(*node, &mut decided) {
            placed[order[block_rank]].push(*node);
        }
    }
    placed
}

struct Late<'a> {
    graph: &'a FunctionGraph,
    rank: &'a HashMap<usize, usize>,
    idom: &'a [usize],
    placement: &'a HashMap<NodeId, usize>,
    early_rank: &'a HashMap<NodeId, usize>,
    users: &'a HashMap<NodeId, Vec<(NodeId, usize)>>,
    pinned: &'a HashSet<NodeId>,
    /// Loop nesting depth per block rank.
    loop_depth: Vec<usize>,
}

impl Late<'_> {
    /// The block rank a pure node is computed in, or `None` when nothing uses it.
    fn block(&self, node: NodeId, decided: &mut HashMap<NodeId, Option<usize>>) -> Option<usize> {
        if let Some(&known) = decided.get(&node) {
            return known;
        }
        let early = self.early_rank[&node];
        let result = if self.pinned.contains(&node) {
            Some(early)
        } else {
            let mut lca = None;
            for &(user, slot) in self.users.get(&node).into_iter().flatten() {
                if let Some(used_in) = self.use_rank(user, slot, decided) {
                    lca = Some(lca.map_or(used_in, |other| self.common_dominator(other, used_in)));
                }
            }
            lca.map(|lca| self.shallowest(early, lca))
        };
        decided.insert(node, result);
        result
    }

    /// Where `user` reads its input `slot`: a Phi reads at the end of the
    /// matching predecessor. Unreachable and unused users read nowhere.
    fn use_rank(
        &self,
        user: NodeId,
        slot: usize,
        decided: &mut HashMap<NodeId, Option<usize>>,
    ) -> Option<usize> {
        if self.early_rank.contains_key(&user) {
            return self.block(user, decided);
        }
        let anchor = if self.graph.op(user) == Opcode::Phi {
            let region = self.graph.input(user, 0);
            self.graph.input(region, slot.checked_sub(1)?)
        } else {
            user
        };
        self.placement.get(&anchor).map(|block| self.rank[block])
    }

    fn common_dominator(&self, mut a: usize, mut b: usize) -> usize {
        while a != b {
            while a > b {
                a = self.idom[a];
            }
            while b > a {
                b = self.idom[b];
            }
        }
        a
    }

    /// The shallowest-loop block on the dominator path from `late` up to `early`.
    fn shallowest(&self, early: usize, late: usize) -> usize {
        let mut best = late;
        let mut cursor = late;
        while cursor != early {
            if cursor == 0 {
                // `early` does not dominate the uses; keep the earliest block.
                return early;
            }
            cursor = self.idom[cursor];
            if self.loop_depth[cursor] < self.loop_depth[best] {
                best = cursor;
            }
        }
        best
    }
}

/// Natural-loop nesting depth per block rank. A back edge targets a block
/// that dominates its source; edges into one header form one loop.
fn loop_depths(
    blocks: &[Block],
    order: &[usize],
    rank: &HashMap<usize, usize>,
    idom: &[usize],
) -> Vec<usize> {
    let mut predecessors = vec![Vec::new(); order.len()];
    for (source, block) in order.iter().enumerate() {
        for successor in &blocks[*block].successors {
            predecessors[rank[successor]].push(source);
        }
    }
    let dominates = |header: usize, mut block: usize| loop {
        if block == header {
            return true;
        }
        if block == 0 {
            return false;
        }
        block = idom[block];
    };
    let mut bodies: HashMap<usize, HashSet<usize>> = HashMap::new();
    for (header, sources) in predecessors.iter().enumerate() {
        for &source in sources {
            if !dominates(header, source) {
                continue;
            }
            let body = bodies.entry(header).or_insert_with(|| HashSet::from([header]));
            let mut work = vec![source];
            while let Some(block) = work.pop() {
                if body.insert(block) {
                    work.extend(predecessors[block].iter().copied());
                }
            }
        }
    }
    let mut depth = vec![0; order.len()];
    for body in bodies.values() {
        for &block in body {
            depth[block] += 1;
        }
    }
    depth
}
