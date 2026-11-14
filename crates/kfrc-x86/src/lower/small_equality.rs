//! Exact-range load plans for an authenticated small byte equality.
use std::sync::OnceLock;

pub(super) fn overlap_enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var("KFRC_SMALL_EQUALITY_OVERLAP").as_deref() == Ok("1"))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Chunk {
    pub offset: u64,
    pub bytes: u8,
}

pub(super) struct Chunks {
    size: u64,
    next: u64,
    overlap_width: Option<u8>,
}

/// Retain the existing 8/4/2/1 plan unless a full-width tail removes chunks.
/// The caller has checked `size <= 32`. Both plans cover exactly [0, size):
/// the overlapping tail ends at size, never at the next power of two.
pub(super) fn plan(size: u64, overlap: bool) -> Chunks {
    debug_assert!(size <= 32);
    let overlap_width = if overlap {
        let width = width(size);
        let greedy_chunks = size / 8 + u64::from((size % 8).count_ones());
        let overlapping_chunks = size.div_ceil(u64::from(width));
        (overlapping_chunks < greedy_chunks).then_some(width)
    } else {
        None
    };
    Chunks {
        size,
        next: 0,
        overlap_width,
    }
}

fn width(remaining: u64) -> u8 {
    match remaining {
        8.. => 8,
        4..=7 => 4,
        2..=3 => 2,
        _ => 1,
    }
}

impl Iterator for Chunks {
    type Item = Chunk;

    fn next(&mut self) -> Option<Self::Item> {
        if self.next >= self.size {
            return None;
        }
        let bytes = self
            .overlap_width
            .unwrap_or_else(|| width(self.size - self.next));
        let offset = if self.overlap_width.is_some() {
            self.next.min(self.size - u64::from(bytes))
        } else {
            self.next
        };
        self.next += u64::from(bytes);
        Some(Chunk { offset, bytes })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overlaps_are_used_only_when_they_remove_chunks() {
        for size in 0..=32 {
            let original: Vec<_> = plan(size, false).collect();
            let candidate: Vec<_> = plan(size, true).collect();
            assert!(candidate.len() <= original.len(), "size={size}");
            if candidate.len() == original.len() {
                assert_eq!(candidate, original, "same-price plans retain the original");
            }
            for chunks in [&original, &candidate] {
                let mut covered = [false; 32];
                for chunk in chunks {
                    let begin = chunk.offset as usize;
                    let end = begin + usize::from(chunk.bytes);
                    assert!(matches!(chunk.bytes, 1 | 2 | 4 | 8));
                    assert!(end <= size as usize, "size={size}; chunk={chunk:?}");
                    covered[begin..end].fill(true);
                }
                assert!(covered[..size as usize].iter().all(|covered| *covered));
                assert!(covered[size as usize..].iter().all(|covered| !covered));
            }
        }
        assert_eq!(
            plan(7, true).collect::<Vec<_>>(),
            vec![
                Chunk {
                    offset: 0,
                    bytes: 4
                },
                Chunk {
                    offset: 3,
                    bytes: 4
                }
            ]
        );
        assert_eq!(
            plan(15, true).collect::<Vec<_>>(),
            vec![
                Chunk {
                    offset: 0,
                    bytes: 8
                },
                Chunk {
                    offset: 7,
                    bytes: 8
                }
            ]
        );
    }

    fn equal(chunks: &[Chunk], left: &[u8], right: &[u8]) -> bool {
        let mut difference = 0u64;
        for chunk in chunks {
            let begin = chunk.offset as usize;
            let end = begin + usize::from(chunk.bytes);
            // Checked, exact-length slices reject any load outside the range.
            let mut a = [0u8; 8];
            let mut b = [0u8; 8];
            a[..usize::from(chunk.bytes)].copy_from_slice(&left[begin..end]);
            b[..usize::from(chunk.bytes)].copy_from_slice(&right[begin..end]);
            difference |= u64::from_le_bytes(a) ^ u64::from_le_bytes(b);
        }
        difference == 0
    }

    #[test]
    fn both_plans_detect_every_changed_byte_at_each_unaligned_base() {
        for overlap in [false, true] {
            for size in 0..=32 {
                let chunks: Vec<_> = plan(size, overlap).collect();
                for base in 0..8 {
                    let mut left = [0u8; 48];
                    let mut right = [0xffu8; 48];
                    let end = base + size as usize;
                    for index in base..end {
                        let byte = (index as u8).wrapping_mul(37).wrapping_add(19);
                        left[index] = byte;
                        right[index] = byte;
                    }
                    // Outside bytes already differ, so they cannot be included.
                    assert!(equal(&chunks, &left[base..end], &right[base..end]));
                    for position in base..end {
                        right[position] ^= 0x80;
                        assert!(!equal(&chunks, &left[base..end], &right[base..end]));
                        assert!(!equal(&chunks, &right[base..end], &left[base..end]));
                        right[position] ^= 0x80;
                    }
                }
            }
        }
    }
}
