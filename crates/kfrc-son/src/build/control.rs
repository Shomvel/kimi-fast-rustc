//! Preserve source enum decisions and attach only established variant facts.

use super::*;
use crate::transforms::control::variants::{case_variant, witness_candidates};
use kfrc_body::SwitchTargets;

impl Builder<'_> {
    /// Recognize the actual Discriminant value, not the local's name or a
    /// representation-layout convention. Cases are mapped using source Rust
    /// discriminants, so nonconsecutive and negative discriminants work too.
    pub(super) fn variant_switch(
        &mut self,
        value: NodeId,
        discriminant_ty: TyId,
        targets: &SwitchTargets,
        block: BlockId,
        control: NodeId,
    ) -> bool {
        let value = self.graph.resolve(value);
        if self.graph.op(value) != Opcode::Discriminant {
            return false;
        }
        let enumeration = self.graph.resolve(self.graph.input(value, 0));
        let Some(ty) = self.graph.rust_type(enumeration) else {
            return false;
        };
        let mut cases = Vec::with_capacity(targets.cases.len());
        for (discriminant, _) in &targets.cases {
            let Some(variant) = case_variant(&self.graph, ty, discriminant_ty, *discriminant)
            else {
                return false;
            };
            if cases.contains(&variant) {
                return false;
            }
            cases.push(variant);
        }
        let payload = self.graph.variant_switches.len() as u32;
        self.graph.variant_switches.push(cases);
        let branch = self.graph.add_node(
            Opcode::VariantSwitch,
            control,
            &[enumeration],
            payload,
            ValueType::Tuple(vec![ValueType::Control; targets.cases.len() + 1]),
        );
        let source = EdgeSource::Block(block);
        for (case, (_, target)) in targets.cases.iter().enumerate() {
            let taken = self.projection(branch, case as u32, ValueType::Control);
            self.add_edge(source, *target, taken);
        }
        let otherwise = self.projection(branch, targets.cases.len() as u32, ValueType::Control);
        self.add_edge(source, targets.otherwise, otherwise);
        true
    }

    /// Delay witness attachment until loop backedges and merge phis are complete.
    /// A MIR downcast alone is not evidence of immediate UB: inactive field
    /// extraction may produce poison that is never observed. Only an actual
    /// dominating switch edge establishes a scoped variant witness.
    pub(super) fn witness_variant_fields(&mut self) {
        witness_candidates(&mut self.graph, &self.variant_witness_candidates);
    }
}
