use super::{HoleAnswer, SpecializationOracle, State};
use crate::{NodeId, Opcode, ValueType};

impl<O: SpecializationOracle> State<'_, O> {
    pub(super) fn copy(&mut self, source: NodeId) -> Result<NodeId, String> {
        if source.is_none() {
            return Ok(NodeId::NONE);
        }
        if let Some(&node) = self.nodes.get(&source) {
            return Ok(node);
        }
        let graph = self.source;
        let op = graph.op(source);
        if op == Opcode::Projection {
            let parent = graph.input(source, 0);
            if let Some(&selected) = self.selected.get(&parent) {
                if graph.payload(source) != selected {
                    return Err("live value depends on a pruned control edge".into());
                }
                let node = self.copy(graph.control(parent))?;
                self.nodes.insert(source, node);
                self.dirty.insert(node);
                return Ok(node);
            }
        }
        // Do not remove a fixed operation merely because its result is known.
        if !op.is_fixed()
            && let Some(value) = self.constant(source)?
        {
            if let ValueType::Rust(ty) = graph.value_type(source) {
                let ty = self.ty(*ty)?;
                let node = self.target.add_constant(value, ty);
                self.nodes.insert(source, node);
                self.stats.nodes_copied += 1;
                if op != Opcode::Const || self.parameter_dependent.contains(&source) {
                    self.dirty.insert(node);
                }
                return Ok(node);
            }
        }
        if op == Opcode::Hole {
            return Err("unprojected or non-scalar hole reached materialization".into());
        }
        let mut op = op;
        let mut payload = graph.payload(source);
        if op == Opcode::TraitCall {
            let call = &graph.trait_calls[payload as usize];
            let HoleAnswer::Resolved { target, identity } = self.answer(call.resolve)? else {
                unreachable!()
            };
            if identity.is_empty() {
                return Err("resolved call lacks a semantic identity".into());
            }
            if target.abi.args.len() != call.signature.args.len() {
                return Err("resolved call requires an ABI argument adapter".into());
            }
            payload = self.target.add_callee(target);
            op = Opcode::Call;
        }
        let ty = self.value_type(graph.value_type(source))?;
        let node = self.target.add_node(op, NodeId::NONE, &[], payload, ty);
        // Allocate first, so loop-carried Phi operands can refer back here.
        self.nodes.insert(source, node);
        self.stats.nodes_copied += 1;
        if self.parameter_dependent.contains(&source) || graph.op(source) == Opcode::TraitCall {
            self.dirty.insert(node);
        }
        let control = self.copy(graph.control(source))?;
        self.target.set_control(node, control);
        let original = graph.inputs(source);
        let kept: Vec<_> = match op {
            Opcode::Region => original
                .iter()
                .copied()
                .filter(|edge| self.reachable.contains(edge))
                .collect(),
            Opcode::Phi => {
                let region = original[0];
                let mut kept = vec![region];
                for (&edge, &value) in graph.inputs(region).iter().zip(&original[1..]) {
                    if self.reachable.contains(&edge) {
                        kept.push(value);
                    }
                }
                kept
            }
            _ => original.to_vec(),
        };
        if kept.len() != original.len() {
            self.dirty.insert(node);
        }
        let inputs = kept
            .iter()
            .map(|input| self.copy(*input))
            .collect::<Result<Vec<_>, _>>()?;
        self.target.set_inputs(node, &inputs);
        match op {
            Opcode::Const | Opcode::ConstRepr => {
                let index = self.target.constants.len() as u32;
                self.target.constants.push(graph.constant_value(source));
                self.target.set_payload(node, index);
            }
            Opcode::VariantField | Opcode::VariantFieldAddr => {
                let index = self.target.variant_fields.len() as u32;
                self.target
                    .variant_fields
                    .push(graph.variant_fields[payload as usize]);
                self.target.set_payload(node, index);
            }
            Opcode::VariantSwitch => {
                let index = self.target.variant_switches.len() as u32;
                self.target
                    .variant_switches
                    .push(graph.variant_switches[payload as usize].clone());
                self.target.set_payload(node, index);
            }
            Opcode::LoadValue
            | Opcode::StoreValue
            | Opcode::SetVariant
            | Opcode::SlotLive
            | Opcode::SlotDead
            | Opcode::CopyNonOverlapping => {
                // Type substitutions may change escape/layout facts. Recompute
                // proven partitions on S rather than copying a stale certificate.
                self.target.set_payload(node, 0);
            }
            Opcode::Call if graph.op(source) == Opcode::Call => {
                return Err("R contains an eagerly resolved call".into());
            }
            Opcode::SymbolAddress | Opcode::ThreadLocalAddress => {
                return Err(
                    "definition symbol constants need an explicit resolution question".into(),
                );
            }
            _ => {}
        }
        Ok(node)
    }
}
