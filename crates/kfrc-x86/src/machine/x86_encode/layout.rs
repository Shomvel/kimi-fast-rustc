//! Joint branch and loop-header layout, in original encoder byte coordinates.
//!
//! Alignment padding can grow when a nearby branch shrinks, invalidating the
//! old shrink-only relaxation rule. Start with short branches and permanently
//! promote every out-of-range branch. Recompute padding after each promotion
//! round. At most `branch_count + 1` layouts are needed; final near branches may
//! be conservative, but every remaining rel8 is checked against the final map.
use super::*;
use std::collections::BTreeSet;

enum EditKind {
    Padding,
    Branch(usize),
}

struct Edit {
    start: usize,
    end: usize,
    size: usize,
    kind: EditKind,
}

struct Layout {
    edits: Vec<Edit>,
    /// At each original byte boundary, its position after all edits there.
    boundaries: Vec<(usize, usize)>,
    size: usize,
}

impl Layout {
    fn new(encoder: &Encoder<'_>, headers: &BTreeSet<usize>) -> Result<Self, EncodeError> {
        let mut edits = Vec::with_capacity(headers.len() + encoder.fixups.len());
        let mut boundaries = vec![(0, 0)];
        let (mut old, mut new) = (0usize, 0usize);
        let mut headers = headers.iter().copied().peekable();
        let mut branches = encoder.fixups.iter().enumerate().peekable();
        let alignment = CODE_ALIGNMENT as usize;
        while headers.peek().is_some() || branches.peek().is_some() {
            // Padding precedes a branch at the same original byte offset, and
            // a label at that offset names the instruction after the padding.
            let padding = headers.peek().is_some_and(|header| {
                branches
                    .peek()
                    .is_none_or(|(_, branch)| *header <= branch.offset)
            });
            let (start, end, size, kind) = if padding {
                let start = headers.next().unwrap();
                new = new
                    .checked_add(start - old)
                    .ok_or(EncodeError::CodeSizeOverflow)?;
                let size = new.wrapping_neg() & (alignment - 1);
                // The per-header budget is alignment - 1, at most 15 B for the
                // default policy. Multiple labels at one address share it.
                (start, start, size, EditKind::Padding)
            } else {
                let (index, branch) = branches.next().unwrap();
                new = new
                    .checked_add(branch.offset - old)
                    .ok_or(EncodeError::CodeSizeOverflow)?;
                let near = if encoder.bytes[branch.offset] == 0xe9 {
                    5
                } else {
                    6
                };
                (
                    branch.offset,
                    branch.offset + near,
                    if branch.short { 2 } else { near },
                    EditKind::Branch(index),
                )
            };
            new = new.checked_add(size).ok_or(EncodeError::CodeSizeOverflow)?;
            old = end;
            boundaries.push((old, new));
            edits.push(Edit {
                start,
                end,
                size,
                kind,
            });
        }
        let size = new
            .checked_add(encoder.bytes.len() - old)
            .ok_or(EncodeError::CodeSizeOverflow)?;
        Ok(Self {
            edits,
            boundaries,
            size,
        })
    }

    fn offset(&self, original: usize) -> usize {
        let index = self.boundaries.partition_point(|&(old, _)| old <= original) - 1;
        let (old, new) = self.boundaries[index];
        new + (original - old)
    }

    fn apply(self, encoder: &mut Encoder<'_>) -> Result<(), EncodeError> {
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(self.size)
            .map_err(|_| EncodeError::AllocationFailed)?;
        let mut old = 0;
        for edit in &self.edits {
            bytes.extend_from_slice(&encoder.bytes[old..edit.start]);
            match edit.kind {
                EditKind::Padding => nops(&mut bytes, edit.size),
                EditKind::Branch(index) => {
                    let branch = &encoder.fixups[index];
                    let unconditional = encoder.bytes[edit.start] == 0xe9;
                    if branch.short {
                        bytes.extend_from_slice(&[
                            if unconditional {
                                0xeb
                            } else {
                                0x70 + (encoder.bytes[edit.start + 1] & 0x0f)
                            },
                            0,
                        ]);
                    } else if unconditional {
                        bytes.extend_from_slice(&[0xe9, 0, 0, 0, 0]);
                    } else {
                        bytes.extend_from_slice(&[0x0f, encoder.bytes[edit.start + 1], 0, 0, 0, 0]);
                    }
                }
            }
            old = edit.end;
        }
        bytes.extend_from_slice(&encoder.bytes[old..]);
        debug_assert_eq!(bytes.len(), self.size);
        for offset in encoder.labels.values_mut() {
            *offset = self.offset(*offset);
        }
        for call in &mut encoder.calls {
            call.offset = self.offset(call.offset as usize) as u64;
        }
        for address in &mut encoder.addresses {
            address.offset = self.offset(address.offset as usize) as u64;
        }
        for branch in &mut encoder.fixups {
            branch.offset = self.offset(branch.offset);
        }
        encoder.bytes = bytes;
        Ok(())
    }
}

pub(super) fn resolve(encoder: &mut Encoder<'_>, loop_headers: &[u32]) -> Result<(), EncodeError> {
    let mut headers = BTreeSet::new();
    for &label in loop_headers {
        if label != EPILOGUE_LABEL {
            headers.insert(
                *encoder
                    .labels
                    .get(&label)
                    .ok_or(EncodeError::UnknownLabel(label))?,
            );
        }
    }
    for branch in &mut encoder.fixups {
        if !encoder.labels.contains_key(&branch.target) {
            return Err(EncodeError::UnknownLabel(branch.target));
        }
        branch.short = true;
    }
    loop {
        let layout = Layout::new(encoder, &headers)?;
        let mut promoted = false;
        for branch in &mut encoder.fixups {
            if branch.short {
                let destination = layout.offset(encoder.labels[&branch.target]);
                let source = layout.offset(branch.offset);
                if i8::try_from(destination as i128 - (source as i128 + 2)).is_err() {
                    branch.short = false;
                    promoted = true;
                }
            }
        }
        if !promoted {
            layout.apply(encoder)?;
            break;
        }
    }
    for branch in &encoder.fixups {
        let target = encoder.labels[&branch.target];
        if branch.short {
            let displacement = i8::try_from(target as i128 - (branch.offset as i128 + 2))
                .map_err(|_| EncodeError::RelocationOverflow(branch.target))?;
            encoder.bytes[branch.offset + 1] = displacement as u8;
        } else {
            let offset = branch.offset
                + if encoder.bytes[branch.offset] == 0xe9 {
                    1
                } else {
                    2
                };
            let displacement = i32::try_from(target as i128 - (offset as i128 + 4))
                .map_err(|_| EncodeError::RelocationOverflow(branch.target))?;
            encoder.bytes[offset..offset + 4].copy_from_slice(&displacement.to_le_bytes());
        }
    }
    Ok(())
}

/// Intel's 1–9 byte NOPs; unlike repeated 0x90, a padding run decodes into at
/// most two instructions at the default budget. NOPs do not alter flags.
fn nops(bytes: &mut Vec<u8>, mut count: usize) {
    const NOPS: [&[u8]; 10] = [
        &[],
        &[0x90],
        &[0x66, 0x90],
        &[0x0f, 0x1f, 0x00],
        &[0x0f, 0x1f, 0x40, 0x00],
        &[0x0f, 0x1f, 0x44, 0x00, 0x00],
        &[0x66, 0x0f, 0x1f, 0x44, 0x00, 0x00],
        &[0x0f, 0x1f, 0x80, 0x00, 0x00, 0x00, 0x00],
        &[0x0f, 0x1f, 0x84, 0x00, 0x00, 0x00, 0x00, 0x00],
        &[0x66, 0x0f, 0x1f, 0x84, 0x00, 0x00, 0x00, 0x00, 0x00],
    ];
    while count != 0 {
        let size = count.min(9);
        bytes.extend_from_slice(NOPS[size]);
        count -= size;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encode<'a>(
        instructions: &[MachineInstr],
        symbols: &'a [String],
        headers: &[u32],
    ) -> Encoder<'a> {
        let mut encoder = Encoder {
            bytes: Vec::new(),
            stack_slots: 0,
            needs_frame: false,
            callee_saved: &[],
            symbols,
            calls: Vec::new(),
            addresses: Vec::new(),
            labels: BTreeMap::new(),
            fixups: Vec::new(),
        };
        for &instruction in instructions {
            encoder.instruction(instruction).unwrap();
        }
        resolve(&mut encoder, headers).unwrap();
        encoder
    }

    fn assert_fixups(encoder: &Encoder<'_>) {
        for branch in &encoder.fixups {
            let (size, displacement) = if branch.short {
                (2, i64::from(encoder.bytes[branch.offset + 1] as i8))
            } else {
                let size = if encoder.bytes[branch.offset] == 0xe9 {
                    5
                } else {
                    6
                };
                let offset = branch.offset + size - 4;
                (
                    size,
                    i64::from(i32::from_le_bytes(
                        encoder.bytes[offset..offset + 4].try_into().unwrap(),
                    )),
                )
            };
            assert_eq!(
                branch.offset as i64 + size as i64 + displacement,
                encoder.labels[&branch.target] as i64
            );
        }
    }

    #[test]
    fn appended_critical_edge_is_not_a_loop() {
        // An acyclic branch is split by allocation. Its edge-copy block is
        // appended after the return and jumps backward into the ordinary join.
        let encoder = encode(
            &[
                MachineInstr::Jcc {
                    condition: Condition::Eq,
                    target: 99,
                },
                MachineInstr::Cqo,
                MachineInstr::Label(1),
                MachineInstr::Ret,
                MachineInstr::Label(99),
                MachineInstr::Mov {
                    dst: Reg::Rax,
                    src: Reg::Rcx,
                },
                MachineInstr::Jmp(1),
            ],
            &[],
            &[],
        );
        assert_eq!(
            encoder.labels[&1], 4,
            "an unmarked backward target must not get padding"
        );
        assert_fixups(&encoder);
    }

    #[test]
    fn padding_and_rel8_boundaries_are_solved_together() {
        for prefix in [0, 1, 2, 7, 15, 16, 17, 30, 31, 32] {
            for body in 120..=132 {
                let mut instructions = vec![MachineInstr::Jmp(3)];
                instructions.extend(std::iter::repeat_n(MachineInstr::Ret, prefix));
                instructions.extend([MachineInstr::Label(1), MachineInstr::Label(2)]);
                instructions.extend(std::iter::repeat_n(MachineInstr::Ret, body));
                instructions.extend([
                    MachineInstr::Jcc {
                        condition: Condition::NotEq,
                        target: 1,
                    },
                    MachineInstr::Jmp(2),
                    MachineInstr::Label(3),
                    MachineInstr::Ret,
                ]);
                let encoder = encode(&instructions, &[], &[1, 2]);
                assert_eq!(encoder.labels[&1] % CODE_ALIGNMENT as usize, 0);
                assert_eq!(encoder.labels[&1], encoder.labels[&2]);
                assert_fixups(&encoder);
                // Header padding is outside the backward displacement: -128
                // still fits when the body is 126, while -129 must promote.
                assert_eq!(encoder.fixups[1].short, body <= 126);
            }
        }
    }

    #[test]
    fn calls_and_address_relocations_follow_padding_and_branch_promotions() {
        let symbols = vec!["callee".into(), "data".into(), "tls".into()];
        let mut instructions = vec![
            MachineInstr::Jmp(2),
            MachineInstr::Cqo,
            MachineInstr::Label(1),
        ];
        instructions.extend([
            MachineInstr::Call { target: 0 },
            MachineInstr::SymbolAddress {
                dst: Reg::R12,
                symbol: 1,
                addend: 19,
            },
            MachineInstr::ThreadLocalAddress {
                dst: Reg::R13,
                symbol: 2,
            },
        ]);
        instructions.extend(std::iter::repeat_n(MachineInstr::Ret, 125));
        instructions.extend([
            MachineInstr::Jcc {
                condition: Condition::NotEq,
                target: 1,
            },
            MachineInstr::Label(2),
            MachineInstr::Ret,
        ]);
        let encoder = encode(&instructions, &symbols, &[1]);
        let header = encoder.labels[&1];
        assert_eq!(header % CODE_ALIGNMENT as usize, 0);
        assert!(!encoder.fixups[0].short && !encoder.fixups[1].short);
        assert_eq!(encoder.calls[0].offset as usize, header + 1);
        assert_eq!(encoder.bytes[header], 0xe8);
        assert_eq!(encoder.addresses[0].offset as usize, header + 8);
        assert_eq!(encoder.addresses[0].kind, AddressRelocationKind::PcRelative);
        assert_eq!(encoder.addresses[0].addend, 19);
        assert_eq!(encoder.addresses[1].offset as usize, header + 24);
        assert_eq!(
            encoder.addresses[1].kind,
            AddressRelocationKind::TlsLocalExec
        );
        for relocation in &encoder.addresses {
            let at = relocation.offset as usize;
            assert_eq!(&encoder.bytes[at..at + 4], &[0, 0, 0, 0]);
        }
        assert_fixups(&encoder);
    }

    #[test]
    fn shared_epilogue_does_not_acquire_loop_padding() {
        let instructions = [
            MachineInstr::Jmp(1),
            MachineInstr::Ret,
            MachineInstr::Label(1),
            MachineInstr::Ret,
        ];
        let saved = [Reg::Rbx];
        let mut encoder = Encoder {
            bytes: Vec::new(),
            stack_slots: 0,
            needs_frame: true,
            callee_saved: &saved,
            symbols: &[],
            calls: Vec::new(),
            addresses: Vec::new(),
            labels: BTreeMap::new(),
            fixups: Vec::new(),
        };
        encoder.prologue(16).unwrap();
        for instruction in instructions {
            encoder.instruction(instruction).unwrap();
        }
        let before = encoder.labels[&EPILOGUE_LABEL];
        resolve(&mut encoder, &[]).unwrap();
        assert_eq!(encoder.labels[&EPILOGUE_LABEL], before - 3);
        assert_fixups(&encoder);
    }

    #[test]
    fn padding_uses_bounded_multibyte_nops() {
        for length in 0..CODE_ALIGNMENT as usize {
            let mut bytes = Vec::new();
            nops(&mut bytes, length);
            assert_eq!(bytes.len(), length);
            if length > 1 {
                assert_ne!(bytes, vec![0x90; length]);
            }
        }
    }
}
