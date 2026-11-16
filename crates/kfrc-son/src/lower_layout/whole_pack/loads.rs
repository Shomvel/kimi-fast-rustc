use super::*;

pub(super) struct Plan {
    pack: pack::Pack,
    pub loads: Vec<NodeId>,
    pub owned: Vec<NodeId>,
    control: NodeId,
    effect: NodeId,
    value_slot: Option<usize>,
}

pub(super) fn plan(
    graph: &FunctionGraph,
    uses: &Uses,
    pack: pack::Pack,
    fuel: &mut Fuel,
) -> Option<Plan> {
    let value_slot = if let Some(region) = pack.region {
        index::forward_join(graph, uses, region, fuel)?;
        let first = pack.lanes[0];
        Some((1..=2).find(|&slot| {
            graph.op(graph.input(first, 3 - slot)) == Opcode::Undefined
                && graph.op(graph.input(first, slot)) == Opcode::Projection
        })?)
    } else {
        None
    };
    let mut loads = Vec::new();
    let mut owned = pack.owned.clone();
    let mut base = NodeId::NONE;
    let mut first_address = NodeId::NONE;
    let mut partition = None;
    let mut next_control = NodeId::NONE;
    let mut next_effect = NodeId::NONE;
    for (offset, &lane) in pack.lanes.iter().enumerate() {
        fuel.take(1)?;
        let value = if let Some(slot) = value_slot {
            let poison = graph.input(lane, 3 - slot);
            let value = graph.input(lane, slot);
            if graph.op(poison) != Opcode::Undefined
                || graph.value_type(poison) != graph.value_type(lane)
                || !graph.control(poison).is_none()
                || !graph.inputs(poison).is_empty()
                || graph.op(value) != Opcode::Projection
                || graph.payload(value) != 2
                || !index::only(uses, value, lane, UseKind::Operand(slot))
            {
                return None;
            }
            value
        } else {
            lane
        };
        let load = graph.input(value, 0);
        if graph.op(load) != Opcode::Load
            || graph.inputs(load).len() != 2
            || graph.value_type(load)
                != &ValueType::Tuple(vec![
                    ValueType::Control,
                    ValueType::Effect,
                    graph.value_type(lane).clone(),
                ])
        {
            return None;
        }
        let address = graph.input(load, 1);
        if offset == 0 {
            first_address = address;
            partition = Some(graph.payload(load));
        } else {
            let pointer = offset_base(graph, address, offset as u128)?;
            if offset == 1 {
                // CSE can erase an identically typed +0 even when the base
                // itself is a PtrOffset. Resolve the group base from byte 1,
                // then accept either the bare base or exactly base+0 first.
                if first_address != pointer && offset_base(graph, first_address, 0) != Some(pointer)
                {
                    return None;
                }
                base = pointer;
            }
            if pointer != base
                || partition != Some(graph.payload(load))
                || graph.control(load) != next_control
                || graph.input(load, 0) != next_effect
                || !index::only(uses, next_control, load, UseKind::Control)
                || !index::only(uses, next_effect, load, UseKind::Operand(0))
            {
                return None;
            }
        }
        next_control = index::projection(graph, uses, load, 0, fuel)?;
        next_effect = index::projection(graph, uses, load, 1, fuel)?;
        if uses.of(load).len() != 3 || index::projection(graph, uses, load, 2, fuel)? != value {
            return None;
        }
        owned.extend([load, value, next_control, next_effect]);
        loads.push(load);
    }
    if let (Some(region), Some(slot)) = (pack.region, value_slot) {
        let edge = graph.input(region, slot - 1);
        if graph.op(edge) != Opcode::Jump
            || !graph.inputs(edge).is_empty()
            || graph.control(edge) != next_control
            || !index::only(uses, next_control, edge, UseKind::Control)
        {
            return None;
        }
        let [usage] = uses.of(next_effect) else {
            return None;
        };
        let effect_phi = usage.user;
        if usage.kind() != UseKind::Operand(slot)
            || graph.op(effect_phi) != Opcode::Phi
            || graph.inputs(effect_phi).len() != 3
            || graph.value_type(effect_phi) != &ValueType::Effect
            || graph.input(effect_phi, 0) != region
        {
            return None;
        }
    }
    Some(Plan {
        pack,
        loads,
        owned,
        control: next_control,
        effect: next_effect,
        value_slot,
    })
}

fn offset_base(graph: &FunctionGraph, address: NodeId, offset: u128) -> Option<NodeId> {
    if graph.op(address) != Opcode::PtrOffset
        || graph.inputs(address).len() != 2
        || !graph.control(address).is_none()
        || graph.payload(address) != 0
    {
        return None;
    }
    let displacement = graph.input(address, 1);
    (graph.op(displacement) == Opcode::Const && graph.constant_value(displacement) == offset)
        .then(|| graph.input(address, 0))
}

impl Plan {
    pub(super) fn phis_removed(&self) -> usize {
        if self.pack.region.is_some() {
            self.loads.len() - 1
        } else {
            0
        }
    }

    pub(super) fn apply(self, graph: &mut FunctionGraph) {
        let first = self.loads[0];
        // Reuse the original address and fixed edge. A scalar's layout size
        // chooses the exact read width; this adds no pointer cast or alignment
        // promise and cannot execute on the inactive predecessor.
        let load = graph.add_node(
            Opcode::Load,
            graph.control(first),
            &[graph.input(first, 0), graph.input(first, 1)],
            graph.payload(first),
            ValueType::Tuple(vec![
                ValueType::Control,
                ValueType::Effect,
                ValueType::Rust(self.pack.ty),
            ]),
        );
        let mut projections = [NodeId::NONE; 3];
        for (slot, ty) in [
            ValueType::Control,
            ValueType::Effect,
            ValueType::Rust(self.pack.ty),
        ]
        .into_iter()
        .enumerate()
        {
            projections[slot] =
                graph.add_node(Opcode::Projection, NodeId::NONE, &[load], slot as u32, ty);
        }
        let value = if let (Some(region), Some(slot)) = (self.pack.region, self.value_slot) {
            let poison = graph.add_node(
                Opcode::Undefined,
                NodeId::NONE,
                &[],
                0,
                ValueType::Rust(self.pack.ty),
            );
            let mut inputs = [region, poison, poison];
            inputs[slot] = projections[2];
            graph.add_node(
                Opcode::Phi,
                NodeId::NONE,
                &inputs,
                0,
                ValueType::Rust(self.pack.ty),
            )
        } else {
            projections[2]
        };
        graph.replace(self.pack.root, value);
        graph.replace(self.control, projections[0]);
        graph.replace(self.effect, projections[1]);
        // Only these fixed reads are removed explicitly. Their dead pure
        // projections/byte Phis/pack cone are pruned by the final compaction.
        for load in self.loads {
            graph.remove_node(load);
        }
    }
}
