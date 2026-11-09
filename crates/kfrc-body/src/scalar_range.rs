//! Producer-certified validity of initialized scalar representations.
//! A range is inclusive and may wrap through zero. It says nothing about
//! initialization, pointer provenance, alignment, or dereferenceability.

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ScalarValidRange {
    bits: u8,
    start: u128,
    end: u128,
}

impl ScalarValidRange {
    pub fn new(bits: u8, start: u128, end: u128) -> Option<Self> {
        if bits == 0 || bits > 128 {
            return None;
        }
        let mask = u128::MAX >> (128 - bits);
        (start <= mask && end <= mask).then_some(Self { bits, start, end })
    }

    pub fn bits(self) -> u8 {
        self.bits
    }
    pub fn start(self) -> u128 {
        self.start
    }
    pub fn end(self) -> u128 {
        self.end
    }
    pub fn mask(self) -> u128 {
        u128::MAX >> (128 - self.bits)
    }
    pub fn is_full(self) -> bool {
        self.end.wrapping_add(1) & self.mask() == self.start
    }
    pub fn contains(self, value: u128) -> bool {
        value <= self.mask()
            && if self.start <= self.end {
                self.start <= value && value <= self.end
            } else {
                value >= self.start || value <= self.end
            }
    }
}

/// Only restricted Initialized lanes are recorded. A missing lane has no
/// validity constraint; in particular Scalar::Union must never become one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ScalarValidity {
    Scalar(ScalarValidRange),
    Pair {
        first: Option<ScalarValidRange>,
        second: Option<ScalarValidRange>,
        second_offset: u64,
    },
}

impl ScalarValidity {
    pub fn lanes(self) -> impl Iterator<Item = (u64, ScalarValidRange)> {
        let lanes = match self {
            Self::Scalar(range) => [Some((0, range)), None],
            Self::Pair {
                first,
                second,
                second_offset,
            } => [
                first.map(|range| (0, range)),
                second.map(|range| (second_offset, range)),
            ],
        };
        lanes.into_iter().flatten()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrapping_ranges_do_not_confuse_full_with_empty() {
        let wrap = ScalarValidRange::new(8, 250, 3).unwrap();
        for value in 0..=255 {
            assert_eq!(wrap.contains(value), value >= 250 || value <= 3);
        }
        assert!(!wrap.contains(256));
        assert!(!wrap.is_full());
        assert!(ScalarValidRange::new(8, 0, 255).unwrap().is_full());
        assert!(ScalarValidRange::new(8, 4, 3).unwrap().is_full());
        assert!(ScalarValidRange::new(128, 0, u128::MAX).unwrap().is_full());
        assert!(ScalarValidRange::new(0, 0, 0).is_none());
        assert!(ScalarValidRange::new(8, 256, 0).is_none());
    }
}
