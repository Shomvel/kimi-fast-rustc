//! Boundary and entry-availability proofs precede every graph mutation.
use super::*;

pub(super) struct Plan {
    pub cells: Vec<Cell>,
    pub groups: BTreeMap<u128, Vec<usize>>,
    pub retired: Vec<NodeId>,
}

fn slot(kind: UseKind) -> usize {
    match kind {
        UseKind::Operand(slot) => slot,
        UseKind::Control => usize::MAX,
    }
}

fn boundary(context: &mut Context<'_>, cells: &[&Cell]) -> Option<HashSet<NodeId>> {
    let graph = context.graph;
    let mut owned = HashSet::default();
    let mut allowed = HashSet::default();
    for cell in cells {
        context.budget.charge(cell.owned.len())?;
        for &node in &cell.owned {
            if !owned.insert(node) {
                return None;
            }
        }
        allowed.insert((cell.success.jump, cell.success.region, cell.success.slot));
        allowed.insert((cell.outgoing, cell.success.effect, cell.success.slot + 1));
    }
    let tail = cells.last()?;
    let terminal = context.control_user(tail.failure)?;
    // The old reject Region and its one Effect Phi remain the continuation.
    // Decline any additional predecessor/value/effect export needing repair.
    let terminal_effect_uses = context.users(tail.failure_effect)?;
    if terminal_effect_uses.len() != 1
        || terminal_effect_uses[0].user != terminal
        || terminal_effect_uses[0].kind != UseKind::Operand(0)
        || !graph.op(terminal).is_fixed()
        || owned.contains(&terminal)
    {
        return None;
    }
    allowed.insert((tail.failure, terminal, usize::MAX));
    allowed.insert((tail.failure_effect, terminal, 0));
    let mut observed = HashSet::default();
    for &node in &owned {
        for usage in context.users(node)? {
            if owned.contains(&usage.user) {
                continue;
            }
            let edge = (node, usage.user, slot(usage.kind));
            if !allowed.contains(&edge) {
                return None;
            }
            observed.insert(edge);
        }
    }
    (observed == allowed).then_some(owned)
}

fn completion(context: &mut Context<'_>, source: NodeId) -> Option<NodeId> {
    let mut result = None;
    for usage in context.users(source)? {
        let node = usage.user;
        if usage.kind == UseKind::Operand(0)
            && context.graph.op(node) == Opcode::Projection
            && context.graph.payload(node) == 0
            && context.graph.value_type(node) == &ValueType::Control
            && result.replace(node).is_some()
        {
            return None;
        }
    }
    result
}

/// Only pure address arithmetic is traversed. A result of a fixed event is
/// available when its completion dominates the new dispatch. Data Phis and
/// anchored arithmetic are deliberately outside this first contract.
fn available(context: &mut Context<'_>, cells: &[&Cell], owned: &HashSet<NodeId>) -> Option<()> {
    let graph = context.graph;
    let head = cells[0];
    context.budget.charge(graph.len())?;
    let cfg = ControlFlow::new(graph);
    let dominators = Dominators::new(&cfg);
    let mut pending = vec![(head.guard.value, false), (head.effect, false)];
    for cell in cells {
        pending.extend(
            graph
                .inputs(cell.event)
                .iter()
                .skip(1)
                .map(|value| (graph.resolve(*value), false)),
        );
    }
    let mut colors = HashMap::<NodeId, u8>::default();
    while let Some((node, exiting)) = pending.pop() {
        context.budget.charge(1)?;
        if exiting {
            colors.insert(node, 2);
            continue;
        }
        match colors.get(&node) {
            Some(1) => return None,
            Some(2) => continue,
            _ => (),
        }
        if colors.len() >= MAX_DEPENDENCIES
            || owned.contains(&node)
            || !graph.control(node).is_none()
        {
            return None;
        }
        colors.insert(node, 1);
        pending.push((node, true));
        match graph.op(node) {
            Opcode::Const | Opcode::SymbolAddress | Opcode::StackSlot
                if graph.inputs(node).is_empty() =>
            {
                ()
            }
            Opcode::PtrOffset if graph.inputs(node).len() == 2 => {
                pending.extend(
                    graph
                        .inputs(node)
                        .iter()
                        .map(|value| (graph.resolve(*value), false)),
                );
            }
            Opcode::Phi
                if node == head.effect
                    && graph.value_type(node) == &ValueType::Effect
                    && context.input(node, 0) == head.entry =>
            {
                ()
            }
            Opcode::Projection if graph.inputs(node).len() == 1 => {
                let source = context.input(node, 0);
                if owned.contains(&source) {
                    return None;
                }
                if source != graph.start {
                    if !graph.op(source).is_fixed() {
                        return None;
                    }
                    let after = completion(context, source)?;
                    if !dominators.dominates(&cfg, after, head.entry) {
                        return None;
                    }
                }
            }
            _ => return None,
        }
    }
    Some(())
}

pub(super) fn collect(context: &mut Context<'_>) -> Option<Plan> {
    let graph = context.graph;
    let mut guards = Vec::new();
    let mut counts = HashMap::<NodeId, usize>::default();
    for node in graph.ids() {
        context.budget.charge(1)?;
        if graph.op(node) == Opcode::If
            && let Some(guard) = context.guard(node)
        {
            *counts.entry(guard.value).or_default() += 1;
            guards.push(guard);
        }
        if context.budget.exhausted {
            return None;
        }
    }
    let mut cells = Vec::new();
    for guard in guards {
        if counts[&guard.value] < 4 {
            continue;
        }
        if let Some(cell) = context.cell(guard) {
            cells.push(cell);
        }
        if context.budget.exhausted {
            return None;
        }
        if cells.len() > MAX_CELLS {
            return None;
        }
    }
    if cells.len() < 4 {
        return None;
    }
    let mut entries = HashMap::default();
    for (index, cell) in cells.iter().enumerate() {
        if entries.insert(cell.entry, index).is_some() {
            return None;
        }
    }
    let mut predecessors = HashSet::default();
    for cell in &cells {
        if let Some(&next) = entries.get(&cell.failure)
            && cells[next].guard.value == cell.guard.value
            && cells[next].effect == cell.failure_effect
        {
            predecessors.insert(next);
        }
    }
    for root in 0..cells.len() {
        if predecessors.contains(&root) {
            continue;
        }
        let mut indices = Vec::new();
        let mut seen = HashSet::default();
        let mut cursor = root;
        loop {
            context.budget.charge(1)?;
            if !seen.insert(cursor) {
                return None;
            }
            indices.push(cursor);
            let cell = &cells[cursor];
            let Some(&next) = entries.get(&cell.failure) else {
                break;
            };
            if cells[next].guard.value != cell.guard.value
                || cells[next].effect != cell.failure_effect
            {
                break;
            }
            cursor = next;
        }
        if indices.len() < 4 {
            continue;
        }
        let chain: Vec<_> = indices.iter().map(|&index| &cells[index]).collect();
        let mut groups = BTreeMap::<u128, Vec<usize>>::new();
        for (index, cell) in chain.iter().enumerate() {
            groups.entry(cell.guard.length).or_default().push(index);
        }
        if groups.len() == chain.len() {
            continue;
        }
        let Some(owned) = boundary(context, &chain) else {
            if context.budget.exhausted {
                return None;
            }
            continue;
        };
        if available(context, &chain, &owned).is_none() {
            if context.budget.exhausted {
                return None;
            }
            continue;
        }
        // Reserve both the bounded construction and retirement before edits.
        let growth = 1 + 2 * (groups.len() + 1);
        context.budget.charge(growth + chain.len() * 9)?;
        let mut retired = Vec::new();
        for (index, cell) in chain.iter().enumerate() {
            retired.extend([
                cell.guard.branch,
                cell.guard.yes,
                cell.guard.no,
                cell.bypass_jump,
                cell.mismatch_jump,
            ]);
            if index + 1 != chain.len() {
                retired.extend([cell.failure, cell.failure_effect]);
            }
        }
        // Moving only the selected cells avoids graph copying. No mutations
        // occur between these immutable proofs and Plan::apply.
        let mut cells_by_index: HashMap<_, _> = cells.into_iter().enumerate().collect();
        let selected = indices
            .into_iter()
            .map(|index| cells_by_index.remove(&index).unwrap())
            .collect();
        return Some(Plan {
            cells: selected,
            groups,
            retired,
        });
    }
    None
}

impl Plan {
    pub(super) fn apply(self, graph: &mut FunctionGraph) {
        let head = &self.cells[0];
        let tail = self.cells.last().unwrap();
        let payload = graph.integer_switches.len() as u32;
        graph
            .integer_switches
            .push(self.groups.keys().copied().collect());
        let switch = graph.add_node(
            Opcode::Switch,
            head.entry,
            &[head.guard.value],
            payload,
            ValueType::Tuple(vec![ValueType::Control; self.groups.len() + 1]),
        );
        let mut reject_controls = Vec::new();
        let mut reject_effects = Vec::new();
        for (slot, group) in self.groups.values().enumerate() {
            let mut control = graph.add_node(
                Opcode::Projection,
                NodeId::NONE,
                &[switch],
                slot as u32,
                ValueType::Control,
            );
            let mut effect = head.effect;
            for &index in group {
                let cell = &self.cells[index];
                graph.set_control(cell.event, control);
                graph.set_input(cell.event, 0, effect);
                control = cell.mismatch;
                effect = cell.outgoing;
            }
            reject_controls.push(graph.add_node(Opcode::Jump, control, &[], 0, ValueType::Control));
            reject_effects.push(effect);
        }
        let default = graph.add_node(
            Opcode::Projection,
            NodeId::NONE,
            &[switch],
            self.groups.len() as u32,
            ValueType::Control,
        );
        reject_controls.push(graph.add_node(Opcode::Jump, default, &[], 0, ValueType::Control));
        reject_effects.push(head.effect);
        graph.set_inputs(tail.failure, &reject_controls);
        let mut inputs = vec![tail.failure];
        inputs.extend(reject_effects);
        graph.set_inputs(tail.failure_effect, &inputs);
        for node in self.retired {
            graph.remove_node(node);
        }
        graph.compact();
    }
}
