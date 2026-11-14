//! kfrc: remove redundant allocator copies across basic-block boundaries.
//!
//! Facts describe equality of current allocation contents, not equality of
//! virtual-register names (whose definitions may execute again in a loop).
//! The forward must analysis intersects those equalities at joins. A backward
//! liveness analysis then discards copies with no machine consumer. Neither
//! analysis changes instructions, operand allocations, or spill-slot offsets.

use alloc::collections::VecDeque;
use alloc::vec;
use alloc::vec::Vec;
use regalloc2::{
    Allocation, Block, Edit, Function, MachineEnv, OperandKind, Output, ProgPoint, RegClass,
};

use crate::FxHashMap;

#[derive(Clone, Copy)]
struct Location {
    allocation: Allocation,
    class: RegClass,
    start: usize,
    end: usize,
}

impl Location {
    fn overlaps(self, other: Self) -> bool {
        if self.allocation.is_reg() || other.allocation.is_reg() {
            self.allocation == other.allocation
        } else {
            self.start < other.end && other.start < self.end
        }
    }

    fn covers(self, other: Self) -> bool {
        self.overlaps(other) && self.start <= other.start && other.end <= self.end
    }
}

#[derive(Clone)]
enum Action {
    Instruction {
        reads: Vec<usize>,
        writes: Vec<usize>,
    },
    Copy {
        edit: usize,
        from: usize,
        to: usize,
    },
}

#[derive(Clone)]
struct Copies {
    locations: Vec<Location>,
    indices: FxHashMap<(Allocation, RegClass), usize>,
    blocks: Vec<Vec<Action>>,
    overlaps: Vec<Vec<usize>>,
    covers: Vec<Vec<usize>>,
}

struct CopyPlacement {
    predecessor: usize,
    position: ProgPoint,
    needed: Vec<(usize, usize)>,
}

#[derive(Default)]
struct CopyCoalescing {
    retargets: Vec<(usize, usize)>,
    insertions: Vec<usize>,
}

impl Copies {
    fn location(&mut self, func: &impl Function, allocation: Allocation, class: RegClass) -> usize {
        *self.indices.entry((allocation, class)).or_insert_with(|| {
            let index = self.locations.len();
            let size = if allocation.is_reg() {
                1
            } else {
                func.spillslot_size(class)
            };
            let start = if allocation.is_stack() && func.multi_spillslot_named_by_last_slot() {
                allocation.index() + 1 - size
            } else {
                allocation.index()
            };
            self.locations.push(Location {
                allocation,
                class,
                start,
                end: start + size,
            });
            index
        })
    }

    fn copy(&mut self, func: &impl Function, edit: usize, data: &Edit) -> Action {
        let Edit::Move { from, to } = *data;
        let class = from
            .as_reg()
            .or_else(|| to.as_reg())
            .expect("allocator stack-to-stack copy")
            .class();
        Action::Copy {
            edit,
            from: self.location(func, from, class),
            to: self.location(func, to, class),
        }
    }

    fn new(func: &impl Function, env: &MachineEnv, output: &Output) -> Self {
        let mut this = Self {
            locations: Vec::new(),
            indices: FxHashMap::default(),
            blocks: Vec::new(),
            overlaps: Vec::new(),
            covers: Vec::new(),
        };
        // Index all locations before collecting clobbers: an implicit clobber
        // matters only when that register participates in the allocation.
        for (_, edit) in &output.edits {
            this.copy(func, 0, edit);
        }
        for block in 0..func.num_blocks() {
            for inst in func.block_insns(Block::new(block)).iter() {
                for (operand, &allocation) in func
                    .inst_operands(inst)
                    .iter()
                    .zip(output.inst_allocs(inst))
                {
                    if allocation.is_some() {
                        this.location(func, allocation, operand.vreg().class());
                    }
                }
            }
        }
        for block in 0..func.num_blocks() {
            let range = func.block_insns(Block::new(block));
            let mut edit = output
                .edits
                .partition_point(|(pos, _)| *pos < ProgPoint::before(range.first()));
            let mut actions = Vec::new();
            for inst in range.iter() {
                while edit < output.edits.len() && output.edits[edit].0 <= ProgPoint::before(inst) {
                    actions.push(this.copy(func, edit, &output.edits[edit].1));
                    edit += 1;
                }
                let mut reads = Vec::new();
                let mut writes = Vec::new();
                for (operand, &allocation) in func
                    .inst_operands(inst)
                    .iter()
                    .zip(output.inst_allocs(inst))
                {
                    if allocation.is_none() {
                        continue;
                    }
                    let location = this.indices[&(allocation, operand.vreg().class())];
                    match operand.kind() {
                        OperandKind::Use => reads.push(location),
                        OperandKind::Def => writes.push(location),
                    }
                }
                for reg in func
                    .inst_clobbers(inst)
                    .into_iter()
                    .chain(env.scratch_by_class.into_iter().flatten())
                {
                    if let Some(&location) = this.indices.get(&(Allocation::reg(reg), reg.class()))
                    {
                        writes.push(location);
                    }
                }
                actions.push(Action::Instruction { reads, writes });
                while edit < output.edits.len() && output.edits[edit].0 <= ProgPoint::after(inst) {
                    actions.push(this.copy(func, edit, &output.edits[edit].1));
                    edit += 1;
                }
            }
            this.blocks.push(actions);
        }
        for &location in &this.locations {
            this.overlaps.push(
                this.locations
                    .iter()
                    .enumerate()
                    .filter_map(|(i, &other)| location.overlaps(other).then_some(i))
                    .collect(),
            );
            this.covers.push(
                this.locations
                    .iter()
                    .enumerate()
                    .filter_map(|(i, &other)| location.covers(other).then_some(i))
                    .collect(),
            );
        }
        this
    }

    fn forward(&self, state: &mut Equalities, action: &Action, retained: &[bool]) {
        match *action {
            Action::Instruction { ref writes, .. } => {
                for &write in writes {
                    for &location in &self.overlaps[write] {
                        state.forget(location);
                    }
                }
            }
            Action::Copy { edit, from, to } if retained[edit] => {
                if !state.equal(from, to) {
                    for &location in &self.overlaps[to] {
                        state.forget(location);
                    }
                    state.copy(from, to);
                }
            }
            _ => {}
        }
    }

    fn backward(&self, live: &mut [bool], action: &Action, retained: &[bool]) -> bool {
        match *action {
            Action::Instruction {
                ref reads,
                ref writes,
            } => {
                for &write in writes {
                    for &location in &self.covers[write] {
                        live[location] = false;
                    }
                }
                for &read in reads {
                    live[read] = true;
                }
            }
            Action::Copy { edit, from, to } if retained[edit] => {
                if !self.overlaps[to].iter().any(|&location| live[location]) {
                    return false;
                }
                for &location in &self.covers[to] {
                    live[location] = false;
                }
                live[from] = true;
            }
            _ => {}
        }
        true
    }

    fn equalities(&self, func: &impl Function, retained: &[bool]) -> Vec<Equalities> {
        let count = self.locations.len();
        let top = Equalities::top(&self.locations);
        let mut outputs = vec![top; self.blocks.len()];
        let mut queue: VecDeque<_> = (0..self.blocks.len()).collect();
        let mut queued = vec![true; self.blocks.len()];
        while let Some(block) = queue.pop_front() {
            queued[block] = false;
            let mut state = self.incoming(func, block, &outputs, count);
            for action in &self.blocks[block] {
                self.forward(&mut state, action, retained);
            }
            if state != outputs[block] {
                outputs[block] = state;
                for successor in func.block_succs(Block::new(block)) {
                    if !queued[successor.index()] {
                        queued[successor.index()] = true;
                        queue.push_back(successor.index());
                    }
                }
            }
        }
        outputs
    }

    fn redundant(&self, func: &impl Function, retained: &mut [bool]) -> bool {
        let outputs = self.equalities(func, retained);
        let mut changed = false;
        for block in 0..self.blocks.len() {
            let mut state = self.incoming(func, block, &outputs, self.locations.len());
            for action in &self.blocks[block] {
                if let Action::Copy { edit, from, to } = *action {
                    if retained[edit] && state.equal(from, to) {
                        retained[edit] = false;
                        changed = true;
                    }
                }
                self.forward(&mut state, action, retained);
            }
        }
        changed
    }

    /// A join's prefix can be redundant on some edges and necessary on
    /// others. Move only allocator copies, across an operand-free branch,
    /// and only when their total static count does not increase. An always
    /// needed copy must not block an independent, partially redundant copy.
    /// Each original edit can move once; new edge edits never move again.
    fn place_prefix(
        &mut self,
        func: &impl Function,
        retained: &mut Vec<bool>,
        edits: &mut Vec<(ProgPoint, Edit)>,
        relocated: &mut Vec<bool>,
    ) -> bool {
        let outputs = self.equalities(func, retained);
        for block in 0..self.blocks.len() {
            if Block::new(block) == func.entry_block() {
                continue;
            }
            let predecessors = func.block_preds(Block::new(block));
            if predecessors.len() < 2 {
                continue;
            }
            let prefix: Vec<_> = self.blocks[block]
                .iter()
                .take_while(|action| matches!(action, Action::Copy { .. }))
                .filter_map(|action| match *action {
                    Action::Copy { edit, from, to } if retained[edit] => Some((edit, from, to)),
                    _ => None,
                })
                .collect();
            if prefix.is_empty() {
                continue;
            }
            let whole = prefix.iter().all(|&(edit, _, _)| !relocated[edit]);
            let selection = whole
                .then(|| self.placement(func, block, &outputs, &prefix, retained))
                .flatten()
                .map(|plans| (prefix.clone(), plans))
                .or_else(|| {
                    prefix.iter().enumerate().find_map(|(index, &copy)| {
                        let (edit, from, to) = copy;
                        if relocated[edit]
                            || prefix[..index]
                                .iter()
                                .any(|&(_, previous_from, previous_to)| {
                                    self.locations[previous_to].overlaps(self.locations[from])
                                        || self.locations[previous_to].overlaps(self.locations[to])
                                        || self.locations[previous_from]
                                            .overlaps(self.locations[to])
                                })
                        {
                            return None;
                        }
                        self.placement(func, block, &outputs, &[copy], retained)
                            .map(|plans| (vec![copy], plans))
                    })
                });
            let Some((selected, plans)) = selection else {
                continue;
            };
            for &(edit, _, _) in &selected {
                retained[edit] = false;
                relocated[edit] = true;
            }
            for plan in plans {
                let at = self.blocks[plan.predecessor].len() - 1;
                let mut actions = Vec::new();
                for (from, to) in plan.needed {
                    let edit = edits.len();
                    edits.push((
                        plan.position,
                        Edit::Move {
                            from: self.locations[from].allocation,
                            to: self.locations[to].allocation,
                        },
                    ));
                    retained.push(true);
                    relocated.push(true);
                    actions.push(Action::Copy { edit, from, to });
                }
                self.blocks[plan.predecessor].splice(at..at, actions);
            }
            return true;
        }
        false
    }

    fn placement(
        &self,
        func: &impl Function,
        block: usize,
        outputs: &[Equalities],
        prefix: &[(usize, usize, usize)],
        retained: &[bool],
    ) -> Option<Vec<CopyPlacement>> {
        let mut plans = Vec::new();
        let mut count = 0;
        let mut redundant_edge = false;
        for predecessor in func.block_preds(Block::new(block)) {
            if func.block_succs(*predecessor) != [Block::new(block)] {
                return None;
            }
            // These are physical reads/writes, including clobbers and scratch
            // registers. The crossed branch cannot observe or change a copy.
            let Some(Action::Instruction { reads, writes }) =
                self.blocks[predecessor.index()].last()
            else {
                return None;
            };
            if !reads.is_empty()
                || prefix.iter().any(|&(_, from, to)| {
                    writes.iter().any(|&write| {
                        self.locations[write].overlaps(self.locations[from])
                            || self.locations[write].overlaps(self.locations[to])
                    })
                })
            {
                return None;
            }
            let terminator = func.block_insns(*predecessor).last();
            if !func.is_branch(terminator) {
                return None;
            }
            let mut state = outputs[predecessor.index()].clone();
            let mut needed = Vec::new();
            for &(edit, from, to) in prefix {
                if !state.equal(from, to) {
                    needed.push((from, to));
                }
                self.forward(&mut state, &Action::Copy { edit, from, to }, retained);
            }
            count += needed.len();
            redundant_edge |= needed.is_empty();
            plans.push(CopyPlacement {
                predecessor: predecessor.index(),
                position: ProgPoint::before(terminator),
                needed,
            });
        }
        (redundant_edge && count <= prefix.len()).then_some(plans)
    }

    fn incoming(
        &self,
        func: &impl Function,
        block: usize,
        outputs: &[Equalities],
        count: usize,
    ) -> Equalities {
        let preds = func.block_preds(Block::new(block));
        if Block::new(block) == func.entry_block() || preds.is_empty() {
            return Equalities::empty(count);
        }
        let mut state = outputs[preds[0].index()].clone();
        for pred in &preds[1..] {
            state.intersect(&outputs[pred.index()]);
        }
        state
    }

    /// Feed a join copy directly from predecessor reloads. The old source
    /// register must be dead afterwards. Real definitions or observations keep
    /// an edge copy; at most one may replace the removed join copy.
    fn coalesce(
        &mut self,
        func: &impl Function,
        retained: &mut Vec<bool>,
        edits: &mut Vec<(ProgPoint, Edit)>,
        relocated: &mut Vec<bool>,
    ) -> bool {
        if !self.blocks.iter().any(|actions| {
            actions
                .iter()
                .take_while(|action| matches!(action, Action::Copy { .. }))
                .any(|action| match *action {
                    Action::Copy { edit, from, to } => {
                        retained[edit]
                            && !relocated[edit]
                            && self.locations[from].allocation.is_reg()
                            && self.locations[to].allocation.is_reg()
                    }
                    _ => false,
                })
        }) {
            return false;
        }
        let inputs = self.live_inputs(func, retained);
        for block in 0..self.blocks.len() {
            let prefix = self.blocks[block]
                .iter()
                .take_while(|action| matches!(action, Action::Copy { .. }))
                .count();
            let mut live = self.outgoing(func, block, &inputs);
            for index in (0..self.blocks[block].len()).rev() {
                if let Action::Copy { edit, from, to } = self.blocks[block][index] {
                    if index < prefix
                        && retained[edit]
                        && !relocated[edit]
                        && self.locations[from].allocation.is_reg()
                        && self.locations[to].allocation.is_reg()
                        && !self.overlaps[from].iter().any(|&location| live[location])
                    {
                        let mut plan = CopyCoalescing::default();
                        if self.coalesce_paths(func, retained, (block, index), from, to, &mut plan)
                            && !plan.retargets.is_empty()
                            && plan.insertions.len() <= 1
                        {
                            retained[edit] = false;
                            relocated[edit] = true;
                            for (producer_block, producer_index) in plan.retargets {
                                let Action::Copy {
                                    edit: producer,
                                    from: origin,
                                    ..
                                } = self.blocks[producer_block][producer_index]
                                else {
                                    unreachable!()
                                };
                                self.blocks[producer_block][producer_index] = Action::Copy {
                                    edit: producer,
                                    from: origin,
                                    to,
                                };
                                // Keep its original program point and order
                                // among other edits at that same point.
                                edits[producer].1 = Edit::Move {
                                    from: self.locations[origin].allocation,
                                    to: self.locations[to].allocation,
                                };
                                relocated[producer] = true;
                            }
                            for predecessor in plan.insertions {
                                let position = ProgPoint::before(
                                    func.block_insns(Block::new(predecessor)).last(),
                                );
                                let edit = edits.len();
                                edits.push((
                                    position,
                                    Edit::Move {
                                        from: self.locations[from].allocation,
                                        to: self.locations[to].allocation,
                                    },
                                ));
                                retained.push(true);
                                relocated.push(true);
                                let at = self.blocks[predecessor].len() - 1;
                                self.blocks[predecessor]
                                    .insert(at, Action::Copy { edit, from, to });
                            }
                            return true;
                        }
                    }
                }
                self.backward(&mut live, &self.blocks[block][index], retained);
            }
        }
        false
    }

    fn coalesce_paths(
        &self,
        func: &impl Function,
        retained: &[bool],
        start: (usize, usize),
        from: usize,
        to: usize,
        plan: &mut CopyCoalescing,
    ) -> bool {
        let mut pending = vec![start];
        let mut visited = vec![false; self.blocks.len()];
        let touches = |location: usize| {
            self.locations[location].overlaps(self.locations[from])
                || self.locations[location].overlaps(self.locations[to])
        };
        'path: while let Some((block, before)) = pending.pop() {
            if visited[block] {
                return false;
            }
            visited[block] = true;
            for index in (0..before).rev() {
                let barrier = match &self.blocks[block][index] {
                    Action::Copy { edit, .. } if !retained[*edit] => false,
                    Action::Copy { to: target, .. } if *target == from => {
                        plan.retargets.push((block, index));
                        continue 'path;
                    }
                    Action::Copy { from, to, .. } => touches(*from) || touches(*to),
                    Action::Instruction { reads, writes } => reads
                        .iter()
                        .chain(writes)
                        .any(|&location| touches(location)),
                };
                if barrier {
                    if block == start.0 || !plan.insertions.is_empty() {
                        return false;
                    }
                    plan.insertions.push(block);
                    continue 'path;
                }
            }
            let predecessors = func.block_preds(Block::new(block));
            if Block::new(block) == func.entry_block() || predecessors.is_empty() {
                return false;
            }
            for predecessor in predecessors {
                if func.block_succs(*predecessor) != [Block::new(block)] {
                    return false;
                }
                let actions = &self.blocks[predecessor.index()];
                let Some(Action::Instruction { reads, writes }) = actions.last() else {
                    return false;
                };
                if !reads.is_empty()
                    || writes.iter().any(|&location| touches(location))
                    || !func.is_branch(func.block_insns(*predecessor).last())
                {
                    return false;
                }
                pending.push((predecessor.index(), actions.len()));
            }
        }
        true
    }

    fn live_inputs(&self, func: &impl Function, retained: &[bool]) -> Vec<Vec<bool>> {
        let mut inputs = vec![vec![false; self.locations.len()]; self.blocks.len()];
        let mut queue: VecDeque<_> = (0..self.blocks.len()).rev().collect();
        let mut queued = vec![true; self.blocks.len()];
        while let Some(block) = queue.pop_front() {
            queued[block] = false;
            let mut live = self.outgoing(func, block, &inputs);
            for action in self.blocks[block].iter().rev() {
                self.backward(&mut live, action, retained);
            }
            if live != inputs[block] {
                inputs[block] = live;
                for predecessor in func.block_preds(Block::new(block)) {
                    if !queued[predecessor.index()] {
                        queued[predecessor.index()] = true;
                        queue.push_back(predecessor.index());
                    }
                }
            }
        }
        inputs
    }

    fn dead(&self, func: &impl Function, retained: &mut [bool]) -> bool {
        let inputs = self.live_inputs(func, retained);
        let mut changed = false;
        for block in 0..self.blocks.len() {
            let mut live = self.outgoing(func, block, &inputs);
            for action in self.blocks[block].iter().rev() {
                if !self.backward(&mut live, action, retained) {
                    if let Action::Copy { edit, .. } = *action {
                        retained[edit] = false;
                        changed = true;
                    }
                }
            }
        }
        changed
    }

    fn outgoing(&self, func: &impl Function, block: usize, inputs: &[Vec<bool>]) -> Vec<bool> {
        let mut live = vec![false; self.locations.len()];
        for successor in func.block_succs(Block::new(block)) {
            for (live, incoming) in live.iter_mut().zip(&inputs[successor.index()]) {
                *live |= incoming;
            }
        }
        live
    }
}

/// A canonical partition of equal contents. Singleton classes carry no fact.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Equalities {
    leaders: Vec<usize>,
    counts: Vec<usize>,
}

impl Equalities {
    fn empty(count: usize) -> Self {
        Self {
            leaders: (0..count).collect(),
            counts: vec![1; count],
        }
    }

    fn top(locations: &[Location]) -> Self {
        let mut result = Self::empty(locations.len());
        let mut classes = [None; 3];
        for (index, location) in locations.iter().enumerate() {
            if let Some(leader) = classes[location.class as usize] {
                result.copy(leader, index);
            } else {
                classes[location.class as usize] = Some(index);
            }
        }
        result
    }

    fn equal(&self, a: usize, b: usize) -> bool {
        self.leaders[a] == self.leaders[b]
    }

    fn forget(&mut self, location: usize) {
        let leader = self.leaders[location];
        if self.counts[leader] == 1 {
            return;
        }
        self.counts[leader] -= 1;
        if leader == location {
            let next = self
                .leaders
                .iter()
                .enumerate()
                .find(|&(index, &value)| index != location && value == leader)
                .unwrap()
                .0;
            for value in &mut self.leaders {
                if *value == leader {
                    *value = next;
                }
            }
            self.counts[next] = self.counts[leader];
        }
        self.leaders[location] = location;
        self.counts[location] = 1;
    }

    fn copy(&mut self, from: usize, to: usize) {
        if self.equal(from, to) {
            return;
        }
        self.forget(to);
        let leader = self.leaders[from];
        let next = leader.min(to);
        let count = self.counts[leader] + 1;
        for value in &mut self.leaders {
            if *value == leader {
                *value = next;
            }
        }
        self.leaders[to] = next;
        self.counts[leader] = 0;
        self.counts[to] = 0;
        self.counts[next] = count;
    }

    fn intersect(&mut self, other: &Self) {
        let mut classes = FxHashMap::default();
        self.counts.fill(0);
        for index in 0..self.leaders.len() {
            let next = *classes
                .entry((self.leaders[index], other.leaders[index]))
                .or_insert(index);
            self.leaders[index] = next;
            self.counts[next] += 1;
        }
    }
}

/// Allocator spill slots are private and are accessed only by these edits or
/// allocated instruction operands. Debug value ranges name additional users;
/// preserve their unmodified allocation output. Fixed stack-register aliases
/// likewise need a target-specific alias contract before this analysis.
/// Register-only edits obey the same physical-content and liveness rules;
/// requiring a spill slot would skip inverse copies around pure register loops.
pub(super) fn run(func: &impl Function, env: &MachineEnv, output: &mut Output) {
    if output.edits.is_empty()
        || !output.debug_locations.is_empty()
        || !env.fixed_stack_slots.is_empty()
    {
        return;
    }
    let mut copies = Copies::new(func, env, output);
    let mut edits = core::mem::take(&mut output.edits);
    let mut retained = vec![true; edits.len()];
    let mut relocated = vec![false; edits.len()];
    loop {
        let redundant = copies.redundant(func, &mut retained);
        let dead = copies.dead(func, &mut retained);
        if !redundant
            && !dead
            && !copies.place_prefix(func, &mut retained, &mut edits, &mut relocated)
            && !copies.coalesce(func, &mut retained, &mut edits, &mut relocated)
        {
            break;
        }
    }
    output.edits = edits
        .into_iter()
        .zip(retained)
        .filter_map(|(edit, retained)| retained.then_some(edit))
        .collect();
    // Stable order retains all original moves, then the relocated prefix at
    // the same pre-branch point, in their original sequential order.
    output.edits.sort_by_key(|&(position, _)| position);
}

#[cfg(test)]
mod tests {
    use super::*;
    use regalloc2::{Inst, InstRange, Operand, PReg, PRegSet, SpillSlot, VReg};

    struct Cfg {
        successors: Vec<Vec<Block>>,
        predecessors: Vec<Vec<Block>>,
        operands: Vec<Vec<Operand>>,
    }

    impl Cfg {
        fn new(edges: &[&[usize]]) -> Self {
            let successors: Vec<Vec<Block>> = edges
                .iter()
                .map(|edges| edges.iter().map(|&b| Block::new(b)).collect())
                .collect();
            let mut predecessors = vec![Vec::new(); edges.len()];
            for (block, succs) in successors.iter().enumerate() {
                for succ in succs {
                    predecessors[succ.index()].push(Block::new(block));
                }
            }
            Self {
                successors,
                predecessors,
                operands: vec![Vec::new(); edges.len()],
            }
        }
    }

    impl Function for Cfg {
        fn num_insts(&self) -> usize {
            self.operands.len()
        }
        fn num_blocks(&self) -> usize {
            self.successors.len()
        }
        fn entry_block(&self) -> Block {
            Block::new(0)
        }
        fn block_insns(&self, block: Block) -> InstRange {
            InstRange::new(Inst::new(block.index()), Inst::new(block.index() + 1))
        }
        fn block_succs(&self, b: Block) -> &[Block] {
            &self.successors[b.index()]
        }
        fn block_preds(&self, b: Block) -> &[Block] {
            &self.predecessors[b.index()]
        }
        fn block_params(&self, _: Block) -> &[VReg] {
            &[]
        }
        fn is_ret(&self, _: Inst) -> bool {
            false
        }
        fn is_branch(&self, _: Inst) -> bool {
            true
        }
        fn branch_blockparams(&self, _: Block, _: Inst, _: usize) -> &[VReg] {
            &[]
        }
        fn inst_operands(&self, inst: Inst) -> &[Operand] {
            &self.operands[inst.index()]
        }
        fn inst_clobbers(&self, _: Inst) -> PRegSet {
            PRegSet::empty()
        }
        fn num_vregs(&self) -> usize {
            0
        }
        fn spillslot_size(&self, class: RegClass) -> usize {
            if class == RegClass::Int { 1 } else { 2 }
        }
    }

    fn register(index: usize, class: RegClass) -> Location {
        let allocation = Allocation::reg(PReg::new(index, class));
        Location {
            allocation,
            class,
            start: allocation.index(),
            end: allocation.index() + 1,
        }
    }

    fn stack(index: usize, class: RegClass) -> Location {
        Location {
            allocation: Allocation::stack(SpillSlot::new(index)),
            class,
            start: index,
            end: index + if class == RegClass::Int { 1 } else { 2 },
        }
    }

    fn copies(locations: Vec<Location>, blocks: Vec<Vec<Action>>) -> Copies {
        let overlaps = locations
            .iter()
            .map(|&a| {
                locations
                    .iter()
                    .enumerate()
                    .filter_map(|(i, &b)| a.overlaps(b).then_some(i))
                    .collect()
            })
            .collect();
        let covers = locations
            .iter()
            .map(|&a| {
                locations
                    .iter()
                    .enumerate()
                    .filter_map(|(i, &b)| a.covers(b).then_some(i))
                    .collect()
            })
            .collect();
        Copies {
            locations,
            indices: FxHashMap::default(),
            blocks,
            overlaps,
            covers,
        }
    }

    fn movement(edit: usize, from: usize, to: usize) -> Action {
        Action::Copy { edit, from, to }
    }
    fn instruction(reads: &[usize], writes: &[usize]) -> Action {
        Action::Instruction {
            reads: reads.to_vec(),
            writes: writes.to_vec(),
        }
    }

    fn optimize(copies: &Copies, cfg: &Cfg, count: usize) -> Vec<bool> {
        let mut retained = vec![true; count];
        loop {
            let forward = copies.redundant(cfg, &mut retained);
            let backward = copies.dead(cfg, &mut retained);
            if !forward && !backward {
                return retained;
            }
        }
    }

    // Compare every original instruction's observed operands over concrete
    // control paths, including repeated loop visits and fresh dynamic writes.
    fn observations(copies: &Copies, path: &[usize], retained: &[bool], seed: u64) -> Vec<u64> {
        let mut values: Vec<_> = copies
            .locations
            .iter()
            .map(|location| seed.wrapping_mul(17) + u64::from(location.allocation.bits()))
            .collect();
        let mut result = Vec::new();
        let mut step = 0;
        for &block in path {
            for action in &copies.blocks[block] {
                match action {
                    Action::Copy { edit, from, to } if retained[*edit] => {
                        values[*to] = values[*from]
                    }
                    Action::Instruction { reads, writes } => {
                        result.extend(reads.iter().map(|&i| values[i]));
                        for &write in writes {
                            values[write] = seed.wrapping_add(
                                step * 31 + u64::from(copies.locations[write].allocation.bits()),
                            );
                        }
                        step += 1;
                    }
                    _ => {}
                }
            }
        }
        result
    }

    fn register_only_env() -> MachineEnv {
        MachineEnv {
            preferred_regs_by_class: [PRegSet::empty(); 3],
            non_preferred_regs_by_class: [PRegSet::empty(); 3],
            scratch_by_class: [None; 3],
            fixed_stack_slots: Vec::new(),
        }
    }

    #[test]
    fn public_cleanup_removes_inverse_register_edits_without_spill_slots() {
        let mut cfg = Cfg::new(&[&[1], &[2], &[]]);
        let r0 = Allocation::reg(PReg::new(0, RegClass::Int));
        let r1 = Allocation::reg(PReg::new(1, RegClass::Int));
        cfg.operands[2].push(Operand::reg_use(VReg::new(0, RegClass::Int)));
        let mut output = Output {
            num_spillslots: 0,
            edits: vec![
                (
                    ProgPoint::before(Inst::new(0)),
                    Edit::Move { from: r0, to: r1 },
                ),
                (
                    ProgPoint::before(Inst::new(1)),
                    Edit::Move { from: r1, to: r0 },
                ),
            ],
            allocs: vec![r0],
            inst_alloc_offsets: vec![0, 0, 0],
            ..Output::default()
        };
        let env = register_only_env();
        let before = Copies::new(&cfg, &env, &output);
        run(&cfg, &env, &mut output);
        assert!(output.edits.is_empty());
        assert_eq!(output.num_spillslots, 0);
        assert_eq!(output.allocs, vec![r0]);
        let after = Copies::new(&cfg, &env, &output);
        for seed in 0..128 {
            assert_eq!(
                observations(&before, &[0, 1, 2], &[true; 2], seed),
                observations(&after, &[0, 1, 2], &[], seed)
            );
        }
    }

    #[test]
    fn register_only_cleanup_keeps_real_writes_and_observed_copies_on_both_paths() {
        let mut cfg = Cfg::new(&[&[1, 2], &[3], &[3], &[]]);
        let r0 = Allocation::reg(PReg::new(0, RegClass::Int));
        let r1 = Allocation::reg(PReg::new(1, RegClass::Int));
        cfg.operands[2].push(Operand::reg_def(VReg::new(0, RegClass::Int)));
        cfg.operands[3].push(Operand::reg_use(VReg::new(1, RegClass::Int)));
        let mut output = Output {
            num_spillslots: 0,
            edits: vec![
                (
                    ProgPoint::before(Inst::new(0)),
                    Edit::Move { from: r0, to: r1 },
                ),
                (
                    ProgPoint::before(Inst::new(3)),
                    Edit::Move { from: r1, to: r0 },
                ),
            ],
            allocs: vec![r1, r0],
            inst_alloc_offsets: vec![0, 0, 0, 1],
            ..Output::default()
        };
        let env = register_only_env();
        let before = Copies::new(&cfg, &env, &output);
        run(&cfg, &env, &mut output);
        let after = Copies::new(&cfg, &env, &output);
        for path in [[0, 1, 3], [0, 2, 3]] {
            for seed in 0..128 {
                assert_eq!(
                    observations(&before, &path, &[true; 2], seed),
                    observations(&after, &path, &vec![true; output.edits.len()], seed)
                );
            }
        }
        assert_eq!(output.num_spillslots, 0);
        assert_eq!(output.allocs, vec![r1, r0]);
    }

    #[test]
    fn removes_inverse_spills_across_a_diamond_and_dead_reloads() {
        let cfg = Cfg::new(&[&[1, 2], &[3], &[3], &[]]);
        let copies = copies(
            vec![register(0, RegClass::Int), stack(0, RegClass::Int)],
            vec![
                vec![],
                vec![movement(0, 1, 0)],
                vec![movement(1, 1, 0)],
                vec![movement(2, 0, 1), instruction(&[1], &[])],
            ],
        );
        let retained = optimize(&copies, &cfg, 3);
        assert_eq!(retained, vec![false; 3]);
        for path in [[0, 1, 3], [0, 2, 3]] {
            for seed in 0..128 {
                assert_eq!(
                    observations(&copies, &path, &[true; 3], seed),
                    observations(&copies, &path, &retained, seed)
                );
            }
        }
    }

    #[test]
    fn a_write_on_one_predecessor_invalidates_the_join_fact() {
        let cfg = Cfg::new(&[&[1, 2], &[3], &[3], &[]]);
        let copies = copies(
            vec![register(0, RegClass::Int), stack(0, RegClass::Int)],
            vec![
                vec![],
                vec![movement(0, 1, 0)],
                vec![movement(1, 1, 0), instruction(&[], &[0])],
                vec![movement(2, 0, 1), instruction(&[1], &[])],
            ],
        );
        let retained = optimize(&copies, &cfg, 3);
        assert!(retained[2]);
        for path in [[0, 1, 3], [0, 2, 3]] {
            for seed in 0..128 {
                assert_eq!(
                    observations(&copies, &path, &[true; 3], seed),
                    observations(&copies, &path, &retained, seed)
                );
            }
        }
    }

    #[test]
    fn loops_do_not_treat_repeated_definitions_as_a_constant_value() {
        let cfg = Cfg::new(&[&[1], &[2, 3], &[1], &[]]);
        let copies = copies(
            vec![
                register(0, RegClass::Int),
                register(1, RegClass::Int),
                stack(0, RegClass::Int),
            ],
            vec![
                vec![movement(0, 0, 2)],
                vec![instruction(&[2], &[])],
                vec![
                    movement(1, 2, 1),
                    movement(2, 1, 2),
                    instruction(&[], &[0]),
                    movement(3, 0, 2),
                ],
                vec![instruction(&[2], &[])],
            ],
        );
        let retained = optimize(&copies, &cfg, 4);
        assert!(retained[0] && retained[3]);
        assert!(!retained[1] && !retained[2]);
        for iterations in 0..32 {
            let mut path = vec![0, 1];
            for _ in 0..iterations {
                path.extend([2, 1]);
            }
            path.push(3);
            for seed in 0..32 {
                assert_eq!(
                    observations(&copies, &path, &[true; 4], seed),
                    observations(&copies, &path, &retained, seed)
                );
            }
        }
    }

    #[test]
    fn partial_spill_overlap_invalidates_equality_but_not_all_liveness() {
        let cfg = Cfg::new(&[&[]]);
        let copies = copies(
            vec![
                register(0, RegClass::Float),
                register(0, RegClass::Int),
                stack(0, RegClass::Float),
                stack(1, RegClass::Int),
            ],
            vec![vec![
                movement(0, 0, 2),
                movement(1, 1, 3),
                movement(2, 2, 0),
                instruction(&[0], &[]),
            ]],
        );
        assert_eq!(optimize(&copies, &cfg, 3), vec![true; 3]);
    }

    #[test]
    fn generated_copy_programs_preserve_observations_across_loop_paths() {
        let cfg = Cfg::new(&[&[1, 2], &[3], &[3], &[1, 2, 4], &[]]);
        let mut random = 0x8975_63ab_ef01_2345u64;
        for case in 0..192 {
            let mut next = || {
                random = random.wrapping_mul(6364136223846793005).wrapping_add(1);
                (random >> 32) as usize
            };
            let locations = vec![
                register(0, RegClass::Int),
                register(1, RegClass::Int),
                register(2, RegClass::Int),
                stack(0, RegClass::Int),
                stack(1, RegClass::Int),
                stack(2, RegClass::Int),
            ];
            let mut edits = 0;
            let mut blocks = Vec::new();
            for block in 0..cfg.num_blocks() {
                let mut actions = Vec::new();
                for _ in 0..8 {
                    let from = next() % locations.len();
                    let to = next() % locations.len();
                    if next() % 3 != 0 {
                        actions.push(movement(edits, from, to));
                        edits += 1;
                    } else {
                        actions.push(instruction(&[from], &[to]));
                    }
                }
                actions.push(instruction(&[(case + block) % locations.len()], &[]));
                blocks.push(actions);
            }
            let copies = copies(locations, blocks);
            let retained = optimize(&copies, &cfg, edits);
            let original = vec![true; edits];
            for pattern in 0..16 {
                let mut path = vec![0];
                for iteration in 0..4 {
                    path.extend([1 + ((pattern >> iteration) & 1), 3]);
                    let mut complete = path.clone();
                    complete.push(4);
                    for seed in [0, 1, 37, 123456789] {
                        assert_eq!(
                            observations(&copies, &complete, &original, seed),
                            observations(&copies, &complete, &retained, seed),
                            "program {case}, path {complete:?}",
                        );
                    }
                }
            }
        }
    }

    fn optimize_placement(copies: &mut Copies, cfg: &Cfg, count: usize) -> Vec<bool> {
        let mut retained = vec![true; count];
        let mut edits = vec![
            (
                ProgPoint::before(Inst::new(0)),
                Edit::Move {
                    from: copies.locations[0].allocation,
                    to: copies.locations[1].allocation
                }
            );
            count
        ];
        let mut relocated = vec![false; count];
        loop {
            let forward = copies.redundant(cfg, &mut retained);
            let backward = copies.dead(cfg, &mut retained);
            if !forward
                && !backward
                && !copies.place_prefix(cfg, &mut retained, &mut edits, &mut relocated)
                && !copies.coalesce(cfg, &mut retained, &mut edits, &mut relocated)
            {
                return retained;
            }
        }
    }

    #[test]
    fn places_partially_redundant_join_copies_on_the_necessary_edge() {
        let cfg = Cfg::new(&[&[1, 2], &[3], &[3], &[]]);
        let mut copies = copies(
            vec![register(0, RegClass::Int), stack(0, RegClass::Int)],
            vec![
                vec![instruction(&[], &[])],
                vec![movement(0, 1, 0), instruction(&[], &[])],
                vec![instruction(&[], &[0]), instruction(&[], &[])],
                vec![movement(1, 0, 1), instruction(&[1], &[])],
            ],
        );
        let before = copies.clone();
        let retained = optimize_placement(&mut copies, &cfg, 2);
        assert_eq!(retained, [false, false, true]);
        for path in [[0, 1, 3], [0, 2, 3]] {
            for seed in 0..128 {
                assert_eq!(
                    observations(&before, &path, &[true; 2], seed),
                    observations(&copies, &path, &retained, seed)
                );
            }
        }
    }

    #[test]
    fn an_always_needed_copy_does_not_block_independent_partial_redundancy() {
        let cfg = Cfg::new(&[&[1, 2], &[3], &[3], &[]]);
        let mut copies = copies(
            vec![
                register(0, RegClass::Int),
                stack(0, RegClass::Int),
                register(1, RegClass::Int),
            ],
            vec![
                vec![instruction(&[], &[])],
                vec![movement(0, 1, 0), instruction(&[], &[])],
                vec![instruction(&[], &[0]), instruction(&[], &[])],
                vec![
                    movement(1, 0, 1),
                    movement(2, 0, 2),
                    instruction(&[1, 2], &[]),
                ],
            ],
        );
        let before = copies.clone();
        let retained = optimize_placement(&mut copies, &cfg, 3);
        assert!(!retained[1], "the store belongs only on the modifying edge");
        assert!(retained.iter().filter(|&&keep| keep).count() <= 3);
        for path in [[0, 1, 3], [0, 2, 3]] {
            for seed in 0..128 {
                assert_eq!(
                    observations(&before, &path, &[true; 3], seed),
                    observations(&copies, &path, &retained, seed)
                );
            }
        }
    }

    #[test]
    fn independent_partial_copies_at_one_join_can_move_separately() {
        let cfg = Cfg::new(&[&[1, 2], &[3], &[3], &[]]);
        let mut copies = copies(
            vec![
                register(0, RegClass::Int),
                stack(0, RegClass::Int),
                register(1, RegClass::Int),
                stack(1, RegClass::Int),
                register(2, RegClass::Int),
            ],
            vec![
                vec![instruction(&[], &[])],
                vec![movement(0, 1, 0), movement(1, 3, 2), instruction(&[], &[])],
                vec![instruction(&[], &[0, 2]), instruction(&[], &[])],
                vec![
                    movement(2, 0, 1),
                    movement(3, 2, 3),
                    movement(4, 0, 4),
                    instruction(&[1, 3, 4], &[]),
                ],
            ],
        );
        let before = copies.clone();
        let retained = optimize_placement(&mut copies, &cfg, 5);
        assert!(!retained[2] && !retained[3]);
        assert!(retained.iter().filter(|&&keep| keep).count() <= 5);
        for path in [[0, 1, 3], [0, 2, 3]] {
            for seed in 0..128 {
                assert_eq!(
                    observations(&before, &path, &[true; 5], seed),
                    observations(&copies, &path, &retained, seed)
                );
            }
        }
    }

    #[test]
    fn partial_placement_preserves_prefix_read_write_dependencies() {
        let cfg = Cfg::new(&[&[1, 2], &[3], &[3], &[]]);
        // RAW, WAR and WAW. Each second edge destroys a pre-prefix equality;
        // moving the dependent copy using that equality would corrupt a read.
        for (prefix, setup, changed, observed) in [
            (vec![(0, 1), (1, 3)], (3, 1), 1, vec![1, 3]),
            (vec![(1, 0), (2, 1)], (1, 2), 2, vec![0, 1]),
            (vec![(0, 1), (1, 3), (2, 1)], (1, 2), 2, vec![1, 3]),
        ] {
            let mut join: Vec<_> = prefix
                .iter()
                .enumerate()
                .map(|(i, &(from, to))| movement(i + 1, from, to))
                .collect();
            join.push(instruction(&observed, &[]));
            let count = prefix.len() + 1;
            let mut copies = copies(
                (0..4).map(|i| register(i, RegClass::Int)).collect(),
                vec![
                    vec![instruction(&[], &[])],
                    vec![
                        movement(0, setup.0, setup.1),
                        instruction(&[setup.1], &[]),
                        instruction(&[], &[]),
                    ],
                    vec![instruction(&[], &[changed]), instruction(&[], &[])],
                    join,
                ],
            );
            let before = copies.clone();
            let mut subset = copies.clone();
            let mut kept = vec![true; count];
            let mut edits = vec![
                (
                    ProgPoint::before(Inst::new(0)),
                    Edit::Move {
                        from: subset.locations[0].allocation,
                        to: subset.locations[1].allocation,
                    },
                );
                count
            ];
            let mut relocated = vec![false; count];
            assert!(
                !subset.place_prefix(&cfg, &mut kept, &mut edits, &mut relocated),
                "dependent prefix {prefix:?} cannot cross earlier copies"
            );
            let retained = optimize_placement(&mut copies, &cfg, count);
            assert!(retained.iter().filter(|&&keep| keep).count() <= count);
            for path in [[0, 1, 3], [0, 2, 3]] {
                for seed in 0..128 {
                    assert_eq!(
                        observations(&before, &path, &vec![true; count], seed),
                        observations(&copies, &path, &retained, seed),
                        "prefix {prefix:?}, path {path:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn coalesces_reload_paths_while_real_definitions_keep_an_edge_copy() {
        let cfg = Cfg::new(&[&[1, 2], &[3], &[3], &[1, 2, 4], &[]]);
        let mut copies = copies(
            vec![
                register(0, RegClass::Int),
                stack(0, RegClass::Int),
                register(1, RegClass::Int),
            ],
            vec![
                vec![instruction(&[], &[])],
                vec![movement(0, 1, 0), instruction(&[], &[])],
                vec![instruction(&[], &[0]), instruction(&[], &[])],
                vec![movement(1, 0, 2), instruction(&[2], &[])],
                vec![instruction(&[2], &[])],
            ],
        );
        let before = copies.clone();
        let retained = optimize_placement(&mut copies, &cfg, 2);
        assert!(
            !retained[1],
            "reloads must feed the final register directly"
        );
        assert!(matches!(copies.blocks[1][0], Action::Copy { to: 2, .. }));
        assert!(retained.iter().filter(|&&keep| keep).count() <= 2);
        for pattern in 0..16 {
            let mut path = vec![0];
            for iteration in 0..4 {
                path.extend([1 + ((pattern >> iteration) & 1), 3]);
                let mut complete = path.clone();
                complete.push(4);
                for seed in [0, 1, 37, 123456789] {
                    assert_eq!(
                        observations(&before, &complete, &[true; 2], seed),
                        observations(&copies, &complete, &retained, seed)
                    );
                }
            }
        }
    }

    #[test]
    fn coalescing_preserves_live_sources_observations_and_other_successors() {
        for (body_reads, branch_reads, exit_reads, escape) in [
            (vec![], vec![], vec![0, 2], false),
            (vec![0], vec![], vec![2], false),
            (vec![2], vec![], vec![2], false),
            (vec![], vec![2], vec![2], false),
            (vec![], vec![], vec![2], true),
        ] {
            let successors: &[usize] = if escape { &[3, 5] } else { &[3] };
            let cfg = Cfg::new(&[&[1, 2], successors, &[3], &[4], &[], &[]]);
            let mut copies = copies(
                vec![
                    register(0, RegClass::Int),
                    stack(0, RegClass::Int),
                    register(1, RegClass::Int),
                ],
                vec![
                    vec![instruction(&[], &[])],
                    vec![
                        movement(0, 1, 0),
                        instruction(&body_reads, &[]),
                        instruction(&branch_reads, &[]),
                    ],
                    vec![instruction(&[], &[0]), instruction(&[], &[])],
                    vec![movement(1, 0, 2), instruction(&[], &[])],
                    vec![instruction(&exit_reads, &[])],
                    vec![instruction(&[0, 2], &[])],
                ],
            );
            let before = copies.clone();
            let retained = optimize_placement(&mut copies, &cfg, 2);
            assert!(retained[1]);
            let mut paths = vec![vec![0, 1, 3, 4], vec![0, 2, 3, 4]];
            if escape {
                paths.push(vec![0, 1, 5]);
            }
            for path in paths {
                for seed in 0..128 {
                    assert_eq!(
                        observations(&before, &path, &[true; 2], seed),
                        observations(&copies, &path, &retained, seed)
                    );
                }
            }
        }
    }

    #[test]
    fn never_places_copies_across_a_branch_operand_or_grows_static_copies() {
        for reads in [&[][..], &[0][..]] {
            let cfg = Cfg::new(&[&[1, 2, 3], &[4], &[4], &[4], &[]]);
            let mut copies = copies(
                vec![register(0, RegClass::Int), stack(0, RegClass::Int)],
                vec![
                    vec![instruction(&[], &[])],
                    vec![movement(0, 1, 0), instruction(reads, &[])],
                    vec![instruction(&[], &[0]), instruction(&[], &[])],
                    vec![instruction(&[], &[0]), instruction(&[], &[])],
                    vec![movement(1, 0, 1), instruction(&[1], &[])],
                ],
            );
            let retained = optimize_placement(&mut copies, &cfg, 2);
            assert_eq!(retained.len(), 2);
            assert!(retained[1]);
        }
    }

    #[test]
    fn generated_edge_placement_preserves_every_observed_operand() {
        let cfg = Cfg::new(&[&[1, 2], &[3], &[3], &[1, 2, 4], &[]]);
        let mut random = 0x5a7b_c8d9_0123_4567u64;
        for case in 0..192 {
            let mut next = || {
                random = random.wrapping_mul(6364136223846793005).wrapping_add(1);
                (random >> 32) as usize
            };
            let locations = vec![
                register(0, RegClass::Int),
                register(1, RegClass::Int),
                register(2, RegClass::Int),
                stack(0, RegClass::Int),
                stack(1, RegClass::Int),
                stack(2, RegClass::Int),
            ];
            let mut edits = 0;
            let mut blocks = Vec::new();
            for block in 0..cfg.num_blocks() {
                let mut actions = Vec::new();
                // Every block begins with a copy prefix; joins and loops can
                // expose partial redundancy after earlier copies disappear.
                for _ in 0..3 {
                    let from = next() % 6;
                    let to = next() % 6;
                    actions.push(movement(edits, from, to));
                    edits += 1;
                }
                for _ in 0..5 {
                    let from = next() % 6;
                    let to = next() % 6;
                    if next() % 3 == 0 {
                        actions.push(instruction(&[from], &[to]));
                    } else {
                        actions.push(movement(edits, from, to));
                        edits += 1;
                    }
                }
                actions.push(instruction(&[(case + block) % 6], &[]));
                actions.push(instruction(&[], &[]));
                blocks.push(actions);
            }
            let mut copies = copies(locations, blocks);
            let before = copies.clone();
            let retained = optimize_placement(&mut copies, &cfg, edits);
            assert!(retained.iter().filter(|&&keep| keep).count() <= edits);
            for pattern in 0..16 {
                let mut path = vec![0];
                for iteration in 0..4 {
                    path.extend([1 + ((pattern >> iteration) & 1), 3]);
                    let mut complete = path.clone();
                    complete.push(4);
                    for seed in [0, 1, 37, 123456789] {
                        assert_eq!(
                            observations(&before, &complete, &vec![true; edits], seed),
                            observations(&copies, &complete, &retained, seed),
                            "program {case}, path {complete:?}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn equality_partitions_remain_canonical_after_kills_and_intersections() {
        let mut a = Equalities::empty(8);
        a.copy(0, 7);
        a.copy(1, 6);
        a.copy(7, 6);
        a.forget(0);
        assert!(a.equal(6, 7));
        assert!(!a.equal(0, 7));
        let mut b = Equalities::empty(8);
        b.copy(6, 7);
        b.copy(1, 0);
        a.intersect(&b);
        assert_eq!(a, b_for_pair(8, 6, 7));
        fn b_for_pair(count: usize, from: usize, to: usize) -> Equalities {
            let mut state = Equalities::empty(count);
            state.copy(from, to);
            state
        }
    }
}
