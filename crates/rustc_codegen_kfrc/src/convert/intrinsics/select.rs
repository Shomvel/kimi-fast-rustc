//! Conditional value selection using the ordinary Body copy contract.

use super::*;
use kfrc_body::SwitchTargets;

impl<'tcx> Converter<'tcx> {
    pub(super) fn intrinsic_select(
        &mut self,
        args: &[Operand],
        destination: Place,
        target: Option<BlockId>,
    ) -> Result<Terminator, String> {
        let target = target.ok_or("select_unpredictable has no successor")?;
        let true_block = BlockId((self.original_blocks + self.pending_blocks.len()) as u32);
        let false_block = BlockId(true_block.0 + 1);
        // MIR has already evaluated all three arguments before this call.
        // Use performs a value copy for scalars, pairs, aggregates and ZSTs;
        // selecting an aggregate address as the result would alias its input.
        // The intrinsic itself does not drop either input. The public hint
        // wrapper supplies the unselected value's destructor separately.
        for value in [args[1], args[2]] {
            self.pending_blocks.push((
                vec![StatementKind::Assign(destination, Rvalue::Use(value))],
                Terminator::Goto(target),
            ));
        }
        Ok(Terminator::SwitchInt {
            discriminant: args[0],
            targets: SwitchTargets {
                cases: vec![(1, true_block)],
                otherwise: false_block,
            },
        })
    }
}
