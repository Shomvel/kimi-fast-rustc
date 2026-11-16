//! Balanced semantic fallback for L switches in the independent x86 backend.
use super::*;

impl Lowering<'_> {
    pub(super) fn emit_switch(&mut self, node: NodeId, block: usize) -> Result<(), LowerError> {
        let graph = self.graph;
        let input = graph.input(node, 0);
        let kind = self.node_kind(input)?;
        let original = self.value(input)?;
        // Switch ordering is on raw bits, independent of signed source type.
        let value = if kind.bits < 64 {
            let unsigned = self.temporary();
            self.canonicalize(
                unsigned,
                original,
                ScalarKind {
                    signed: false,
                    ..kind
                },
            );
            VirtualValue::Reg(unsigned)
        } else {
            original
        };
        let successors = &self.schedule.blocks[block].successors;
        let mut cases: Vec<_> = graph.integer_switches[graph.payload(node) as usize]
            .iter()
            .zip(successors)
            .map(|(&bits, &target)| (bits as u64, label_of(target)))
            .collect();
        cases.sort_unstable_by_key(|case| case.0);
        let otherwise = label_of(successors[cases.len()]);
        self.switch_tree(value, &cases, otherwise)
    }

    fn switch_label(&mut self) -> Result<u32, LowerError> {
        let label = self.next_switch_label;
        self.next_switch_label = label
            .checked_add(1)
            .ok_or_else(|| LowerError::Unsupported("label space exhausted".into()))?;
        Ok(label)
    }

    fn switch_tree(
        &mut self,
        value: VirtualValue,
        cases: &[(u64, u32)],
        otherwise: u32,
    ) -> Result<(), LowerError> {
        if cases.len() <= 3 {
            for &(bits, target) in cases {
                self.push(VirtualInstr::Test {
                    lhs: value,
                    rhs: VirtualValue::Imm(bits as i64),
                });
                self.push(VirtualInstr::Jcc {
                    condition: Condition::Eq,
                    target,
                });
            }
            self.push(VirtualInstr::Jmp(otherwise));
            return Ok(());
        }
        let middle = cases.len() / 2;
        let (bits, target) = cases[middle];
        self.push(VirtualInstr::Test {
            lhs: value,
            rhs: VirtualValue::Imm(bits as i64),
        });
        self.push(VirtualInstr::Jcc {
            condition: Condition::Eq,
            target,
        });
        let left = self.switch_label()?;
        // Re-emit the compare: allocator edge copies preserve flags, but a
        // multi-branch stream must not depend on a previous block's flags.
        self.push(VirtualInstr::Test {
            lhs: value,
            rhs: VirtualValue::Imm(bits as i64),
        });
        self.push(VirtualInstr::Jcc {
            condition: Condition::UnsignedBelow,
            target: left,
        });
        self.switch_tree(value, &cases[middle + 1..], otherwise)?;
        self.push(VirtualInstr::Label(left));
        self.switch_tree(value, &cases[..middle], otherwise)
    }
}
