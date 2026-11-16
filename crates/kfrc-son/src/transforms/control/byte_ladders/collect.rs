//! Match fixed comparison cells and their exact reconvergence receipts.
use super::*;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Merges {
    EffectOnly,
    EffectAndValues,
}

impl Context<'_> {
    pub(super) fn guard(&mut self, branch: NodeId) -> Option<Guard> {
        let graph = self.graph;
        if graph.op(branch) != Opcode::If || graph.inputs(branch).len() != 1 {
            return None;
        }
        let (condition, inverted, _) = self.peel(self.input(branch, 0), &[])?;
        if graph.op(condition) != Opcode::Compare || graph.inputs(condition).len() != 2 {
            return None;
        }
        let equal = match ComparePredicate::from_payload(graph.payload(condition)) {
            ComparePredicate::Equal => true,
            ComparePredicate::NotEqual => false,
            _ => return None,
        } ^ inverted;
        let (mut value, mut constant) = (self.input(condition, 0), self.input(condition, 1));
        if graph.op(value) == Opcode::Const {
            std::mem::swap(&mut value, &mut constant);
        }
        let length = self.constant(constant)?;
        if !(1..=32).contains(&length)
            || !self.ordinary_int(value, IntegerWidth::Pointer, false)
            || graph.value_type(value) != graph.value_type(constant)
        {
            return None;
        }
        let edges = self.projections::<2>(branch)?;
        if edges
            .iter()
            .any(|edge| graph.value_type(*edge) != &ValueType::Control)
        {
            return None;
        }
        Some(Guard {
            branch,
            value,
            length,
            yes: edges[usize::from(!equal)],
            no: edges[usize::from(equal)],
        })
    }

    /// Peel only bounded boolean inversions and single-input Phis belonging
    /// to this event's result spine. Guard matching passes no Phi regions.
    fn peel(
        &mut self,
        mut value: NodeId,
        regions: &[NodeId],
    ) -> Option<(NodeId, bool, Vec<NodeId>)> {
        let mut inverted = false;
        let mut owned = Vec::new();
        for _ in 0..MAX_WRAPPERS {
            self.budget.charge(1)?;
            if !self.boolean(value) || !self.graph.control(value).is_none() {
                return None;
            }
            match self.graph.op(value) {
                Opcode::BitNot if self.graph.inputs(value).len() == 1 => {
                    inverted = !inverted;
                }
                Opcode::Phi
                    if self.graph.inputs(value).len() == 2
                        && self.graph.payload(value) == 0
                        && regions.contains(&self.input(value, 0)) =>
                {
                    ()
                }
                _ => return Some((value, inverted, owned)),
            }
            owned.push(value);
            value = self.input(value, usize::from(self.graph.op(value) == Opcode::Phi));
        }
        None
    }

    fn join(&mut self, edge: NodeId) -> Option<(NodeId, NodeId, usize)> {
        let edge_uses = self.users(edge)?;
        let [usage] = edge_uses.as_slice() else {
            return None;
        };
        let jump = usage.user;
        if usage.kind() != UseKind::Control
            || self.graph.op(jump) != Opcode::Jump
            || !self.graph.inputs(jump).is_empty()
        {
            return None;
        }
        let jump_uses = self.users(jump)?;
        let [usage] = jump_uses.as_slice() else {
            return None;
        };
        let UseKind::Operand(slot) = usage.kind() else {
            return None;
        };
        let region = usage.user;
        if self.graph.op(region) != Opcode::Region || !self.graph.control(region).is_none() {
            return None;
        }
        Some((jump, region, slot))
    }

    /// The single Effect Phi of `region`. Value Phis are admitted only where
    /// `values` holds: a success target keeps its predecessor slots, so their
    /// incoming values stay in place, whereas a failure join is rebuilt or
    /// retired and admits no value merge.
    fn effect_phi(&mut self, region: NodeId, values: Merges) -> Option<NodeId> {
        let mut phi = None;
        for usage in self.users(region)? {
            if usage.kind() == UseKind::Control {
                continue;
            }
            let node = usage.user;
            if usage.kind() != UseKind::Operand(0)
                || self.graph.op(node) != Opcode::Phi
                || !self.graph.control(node).is_none()
                || self.graph.payload(node) != 0
                || self.graph.inputs(node).len() != self.graph.inputs(region).len() + 1
            {
                return None;
            }
            if self.graph.value_type(node) != &ValueType::Effect {
                if values == Merges::EffectOnly {
                    return None;
                }
                continue;
            }
            if phi.replace(node).is_some() {
                return None;
            }
        }
        phi
    }

    /// Walk empty control wrappers, retaining their ordinary single-input
    /// bool/Effect Phis. Any extra fixed/anchored event stops collection.
    fn result_spine(&mut self, mut control: NodeId) -> Option<(NodeId, Vec<NodeId>, Vec<NodeId>)> {
        let mut owned = Vec::new();
        let mut regions = Vec::new();
        for _ in 0..MAX_WRAPPERS {
            let next = self.control_user(control)?;
            if self.graph.op(next) == Opcode::If {
                return Some((next, owned, regions));
            }
            let (jump, region, _) = self.join(control)?;
            if self.graph.inputs(region) != [jump] {
                return None;
            }
            owned.extend([jump, region]);
            regions.push(region);
            for usage in self.users(region)? {
                if usage.kind() == UseKind::Control {
                    continue;
                }
                let phi = usage.user;
                if usage.kind() != UseKind::Operand(0)
                    || self.graph.op(phi) != Opcode::Phi
                    || self.graph.payload(phi) != 0
                    || !self.graph.control(phi).is_none()
                    || self.graph.inputs(phi).len() != 2
                    || !(self.graph.value_type(phi) == &ValueType::Effect || self.boolean(phi))
                {
                    return None;
                }
                owned.push(phi);
            }
            control = region;
        }
        None
    }

    fn success(
        &mut self,
        mut control: NodeId,
        outgoing: NodeId,
        owned: &mut Vec<NodeId>,
    ) -> Option<Target> {
        for _ in 0..MAX_WRAPPERS {
            let (jump, region, slot) = self.join(control)?;
            let users = self.users(region)?;
            let next = self.control_user(region)?;
            if self.graph.inputs(region) == [jump]
                && users.len() == 1
                && self.graph.op(next) == Opcode::Jump
            {
                owned.extend([jump, region]);
                control = region;
                continue;
            }
            let effect = self.effect_phi(region, Merges::EffectAndValues)?;
            if self.input(effect, slot + 1) != outgoing {
                return None;
            }
            owned.push(jump);
            return Some(Target {
                region,
                effect,
                slot,
                jump,
            });
        }
        None
    }

    fn event_type(&self, event: NodeId, guard: &Guard) -> Option<bool> {
        let graph = self.graph;
        let ValueType::Tuple(fields) = graph.value_type(event) else {
            return None;
        };
        let [
            ValueType::Control,
            ValueType::Effect,
            ValueType::Rust(result),
        ] = fields.as_slice()
        else {
            return None;
        };
        if graph.oracle.scalar_validity(*result).is_some() {
            return None;
        }
        match graph.op(event) {
            Opcode::Call
                if graph.inputs(event).len() == 4
                    && graph.runtime_comparison(graph.payload(event))
                        == Some(RuntimeComparison::Bcmp)
                    && self.constant(self.input(event, 3)) == Some(guard.length)
                    && graph.value_type(self.input(event, 3)) == graph.value_type(guard.value)
                    && matches!(
                        graph.oracle.shape(*result),
                        TypeShape::Int {
                            width: IntegerWidth::W32,
                            signed: true
                        }
                    )
                    // The producer passes literal and input addresses as
                    // thin references (`&[u8; N]`) or raw pointers.
                    && [1, 2].into_iter().all(|slot| {
                        graph.rust_type(self.input(event, slot)).is_some_and(|ty| {
                            matches!(
                                graph.oracle.shape(ty),
                                TypeShape::RawPointer { .. } | TypeShape::Reference { .. }
                            )
                        })
                    }) =>
            {
                Some(false)
            }
            Opcode::Load
                if graph.inputs(event).len() == 2
                    && guard.length == 1
                    && matches!(
                        graph.oracle.shape(*result),
                        TypeShape::Int {
                            width: IntegerWidth::W8,
                            signed: false
                        }
                    ) =>
            {
                Some(true)
            }
            _ => None,
        }
    }

    fn result_equality(
        &mut self,
        branch: NodeId,
        result: NodeId,
        byte: bool,
        regions: &[NodeId],
        owned: &mut Vec<NodeId>,
    ) -> Option<bool> {
        if self.graph.inputs(branch).len() != 1 {
            return None;
        }
        let (condition, inverted, peeled) = self.peel(self.input(branch, 0), regions)?;
        owned.extend(peeled);
        if self.graph.op(condition) != Opcode::Compare || self.graph.inputs(condition).len() != 2 {
            return None;
        }
        let equal = match ComparePredicate::from_payload(self.graph.payload(condition)) {
            ComparePredicate::Equal => true,
            ComparePredicate::NotEqual => false,
            _ => return None,
        } ^ inverted;
        let (mut value, mut zero) = (self.input(condition, 0), self.input(condition, 1));
        if self.constant(value) == Some(0) {
            std::mem::swap(&mut value, &mut zero);
        }
        if self.constant(zero) != Some(0)
            || self.graph.value_type(value) != self.graph.value_type(zero)
        {
            return None;
        }
        owned.push(condition);
        if !byte {
            return (value == result).then_some(equal);
        }
        if self.graph.op(value) != Opcode::Sub
            || self.graph.payload(value) != 0
            || !self.graph.control(value).is_none()
            || self.graph.inputs(value).len() != 2
            || !self.ordinary_int(value, IntegerWidth::W32, true)
        {
            return None;
        }
        let widened = self.input(value, 0);
        let literal = self.input(value, 1);
        if !self.constant(literal).is_some_and(|byte| byte <= 255)
            || self.graph.value_type(literal) != self.graph.value_type(value)
            || self.graph.op(widened) != Opcode::IntCast
            || self.graph.payload(widened) != 0
            || !self.graph.control(widened).is_none()
            || self.graph.inputs(widened).len() != 1
            || self.input(widened, 0) != result
            || self.graph.value_type(widened) != self.graph.value_type(value)
        {
            return None;
        }
        owned.extend([value, widened]);
        Some(equal)
    }

    pub(super) fn cell(&mut self, guard: Guard) -> Option<Cell> {
        let graph = self.graph;
        let event = self.control_user(guard.yes)?;
        let byte = self.event_type(event, &guard)?;
        let [control, effect, result] = self.projections::<3>(event)?;
        if graph.value_type(control) != &ValueType::Control
            || graph.value_type(effect) != &ValueType::Effect
        {
            return None;
        }
        let (branch, mut owned, regions) = self.result_spine(control)?;
        let equal = self.result_equality(branch, result, byte, &regions, &mut owned)?;
        let edges = self.projections::<2>(branch)?;
        if edges
            .iter()
            .any(|edge| graph.value_type(*edge) != &ValueType::Control)
        {
            return None;
        }
        let yes = edges[usize::from(!equal)];
        let mismatch = edges[usize::from(equal)];
        let (bypass_jump, failure, bypass_slot) = self.join(guard.no)?;
        let (mismatch_jump, same_failure, mismatch_slot) = self.join(mismatch)?;
        if same_failure != failure || graph.inputs(failure).len() != 2 {
            return None;
        }
        let failure_effect = self.effect_phi(failure, Merges::EffectOnly)?;
        let input_effect = self.input(event, 0);
        let outgoing = self.input(failure_effect, mismatch_slot + 1);
        if graph.value_type(input_effect) != &ValueType::Effect
            || self.input(failure_effect, bypass_slot + 1) != input_effect
        {
            return None;
        }
        let mut cursor = outgoing;
        for _ in 0..=MAX_WRAPPERS {
            self.budget.charge(1)?;
            if cursor == effect {
                break;
            }
            if graph.op(cursor) != Opcode::Phi
                || graph.value_type(cursor) != &ValueType::Effect
                || graph.inputs(cursor).len() != 2
                || !regions.contains(&self.input(cursor, 0))
            {
                return None;
            }
            cursor = self.input(cursor, 1);
        }
        if cursor != effect {
            return None;
        }
        let success = self.success(yes, outgoing, &mut owned)?;
        owned.extend([
            guard.branch,
            guard.yes,
            guard.no,
            event,
            control,
            effect,
            result,
            branch,
            edges[0],
            edges[1],
            bypass_jump,
            mismatch_jump,
            failure,
            failure_effect,
        ]);
        owned.sort_unstable();
        owned.dedup();
        Some(Cell {
            entry: graph.resolve(graph.control(guard.branch)),
            guard,
            event,
            effect: input_effect,
            outgoing,
            mismatch,
            failure,
            failure_effect,
            bypass_jump,
            mismatch_jump,
            success,
            owned,
        })
    }
}
