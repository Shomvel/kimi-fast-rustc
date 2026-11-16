//! Select sparse search trees and dense jump tables with Cranelift's helper.
use super::*;
use cranelift_frontend::Switch;

impl Exporter<'_, '_> {
    pub(super) fn emit_switch(&mut self, node: NodeId, block: usize) -> Result<(), LowerError> {
        let cases = &self.graph.integer_switches[self.graph.payload(node) as usize];
        let successors = &self.schedule.blocks[block].successors;
        let input = self.graph.input(node, 0);
        let bits = self.scalar_of_node(input)?.bits;
        let value = self.operand(input, bits)?;
        let mut switch = Switch::new();
        for (&case, &successor) in cases.iter().zip(successors) {
            switch.set_entry(case, self.blocks[successor]);
        }
        switch.emit(
            &mut self.builder,
            value,
            self.blocks[successors[cases.len()]],
        );
        Ok(())
    }
}
