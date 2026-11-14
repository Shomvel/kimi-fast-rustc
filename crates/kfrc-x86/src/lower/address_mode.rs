//! Select each graph memory address once; keep both register operands explicit.
use super::*;

const ADDRESS_STEPS: usize = 32;

struct Selection {
    address: MemoryAddress<NodeId>,
    covered: Vec<NodeId>,
}

impl Lowering<'_> {
    fn select_address(&self, address: NodeId) -> Selection {
        let graph = self.graph;
        let mut selected = Selection {
            address: MemoryAddress {
                base: address,
                index: None,
                offset: 0,
            },
            covered: Vec::new(),
        };
        let mut fuel = ADDRESS_STEPS;
        while fuel > 0 {
            let node = selected.address.base;
            if graph.op(node) != Opcode::PtrOffset || !graph.control(node).is_none() {
                break;
            }
            fuel -= 1;
            let offset = graph.input(node, 1);
            if let Ok(step) = self.constant(offset) {
                // An unrepresentable constant stays pointer arithmetic; it
                // must not silently turn into a register-index candidate.
                let Some(displacement) = add_displacement(selected.address.offset, step) else {
                    break;
                };
                selected.address.offset = displacement;
            } else if selected.address.index.is_none() {
                let (index, scale, displacement, covered) =
                    self.select_index(offset, selected.address.offset, &mut fuel);
                selected.address.index = Some((index, scale));
                selected.address.offset = displacement;
                selected.covered.extend(covered);
            } else {
                break;
            }
            selected.covered.push(node);
            selected.address.base = graph.input(node, 0);
        }
        selected
    }

    fn select_index(
        &self,
        mut index: NodeId,
        mut displacement: i32,
        fuel: &mut usize,
    ) -> (NodeId, Scale, i32, Vec<NodeId>) {
        let graph = self.graph;
        let mut covered = Vec::new();
        // Only full-width, unanchored arithmetic may disappear. Narrow
        // operations retain their truncation/extension before becoming an
        // index. On defined L executions these 64-bit operations have the
        // same modulo-64 result as the SIB address calculation.
        while *fuel > 0 && self.address_arithmetic(index) {
            let op = graph.op(index);
            let lhs = graph.input(index, 0);
            let rhs = graph.input(index, 1);
            let step = match op {
                Opcode::Add => self
                    .constant(rhs)
                    .map(|step| (lhs, step))
                    .or_else(|_| self.constant(lhs).map(|step| (rhs, step)))
                    .ok(),
                Opcode::Sub => self
                    .constant(rhs)
                    .ok()
                    .and_then(i64::checked_neg)
                    .map(|step| (lhs, step)),
                _ => None,
            };
            let Some((next, step)) = step else {
                break;
            };
            let Some(next_displacement) = add_displacement(displacement, step) else {
                break;
            };
            *fuel -= 1;
            covered.push(index);
            displacement = next_displacement;
            index = next;
        }
        let mut scale = Scale::One;
        if *fuel > 0 && self.address_arithmetic(index) {
            let lhs = graph.input(index, 0);
            let rhs = graph.input(index, 1);
            let scaled = match graph.op(index) {
                Opcode::Mul => self
                    .constant(rhs)
                    .map(|factor| (lhs, factor))
                    .or_else(|_| self.constant(lhs).map(|factor| (rhs, factor)))
                    .ok()
                    .and_then(|(index, factor)| scale_for(factor).map(|scale| (index, scale))),
                Opcode::ShiftLeft => self.constant(rhs).ok().and_then(|amount| match amount {
                    0..=3 => scale_for(1 << amount).map(|scale| (lhs, scale)),
                    _ => None,
                }),
                _ => None,
            };
            if let Some((next, selected_scale)) = scaled {
                // The input image must also be full width; a narrower input
                // has its own canonical representation which cannot be
                // replaced by truncating just the effective address.
                if self.node_kind(next).is_ok_and(|kind| kind.bits == 64) {
                    *fuel -= 1;
                    covered.push(index);
                    index = next;
                    scale = selected_scale;
                }
            }
        }
        (index, scale, displacement, covered)
    }

    fn address_arithmetic(&self, node: NodeId) -> bool {
        self.graph.control(node).is_none()
            && matches!(
                self.graph.op(node),
                Opcode::Add | Opcode::Sub | Opcode::Mul | Opcode::ShiftLeft
            )
            && self.node_kind(node).is_ok_and(|kind| kind.bits == 64)
    }

    /// A covered node is omitted only when every user is covered too, or
    /// directly consumes this node as a selected memory address. Required
    /// base/index leaves and materialized users keep their dependencies.
    pub(super) fn select_address_modes(
        &self,
    ) -> (HashMap<NodeId, MemoryAddress<NodeId>>, HashSet<NodeId>) {
        let graph = self.graph;
        let mut selections = HashMap::default();
        let mut candidates = HashSet::default();
        let mut required = HashSet::default();
        for node in graph.ids() {
            if matches!(graph.op(node), Opcode::Load | Opcode::Store) {
                let selected = self.select_address(graph.input(node, 1));
                candidates.extend(selected.covered.iter().copied());
                selected.address.map(|node| {
                    required.insert(node);
                });
                selections.insert(node, selected);
            }
        }
        let mut users: HashMap<NodeId, Vec<(NodeId, usize)>> = HashMap::default();
        for node in graph.ids() {
            for (slot, &input) in graph.inputs(node).iter().enumerate() {
                users
                    .entry(graph.resolve(input))
                    .or_default()
                    .push((node, slot));
            }
            let control = graph.control(node);
            if !control.is_none() {
                required.insert(graph.resolve(control));
            }
        }
        let mut rejected = Vec::new();
        for &node in &candidates {
            if required.contains(&node)
                || users.get(&node).is_some_and(|users| {
                    users.iter().any(|&(user, slot)| {
                        !candidates.contains(&user)
                            && !(slot == 1
                                && selections
                                    .get(&user)
                                    .is_some_and(|selected| selected.covered.contains(&node)))
                    })
                })
            {
                rejected.push(node);
            }
        }
        while let Some(node) = rejected.pop() {
            if candidates.remove(&node) {
                rejected.extend(
                    graph
                        .inputs(node)
                        .iter()
                        .map(|&input| graph.resolve(input))
                        .filter(|input| candidates.contains(input)),
                );
            }
        }
        (
            selections
                .into_iter()
                .map(|(node, selected)| (node, selected.address))
                .collect(),
            candidates,
        )
    }
}

fn add_displacement(current: i32, step: i64) -> Option<i32> {
    i64::from(current)
        .checked_add(step)
        .and_then(|value| i32::try_from(value).ok())
}

fn scale_for(factor: i64) -> Option<Scale> {
    match factor {
        1 => Some(Scale::One),
        2 => Some(Scale::Two),
        4 => Some(Scale::Four),
        8 => Some(Scale::Eight),
        _ => None,
    }
}
