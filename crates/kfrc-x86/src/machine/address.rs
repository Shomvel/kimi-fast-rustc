//! The same address operands survive selection, allocation, and encoding.

/// The four index scales representable by an x86 SIB byte.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Scale {
    One,
    Two,
    Four,
    Eight,
}

impl Scale {
    pub fn factor(self) -> u8 {
        1 << self.encoding()
    }

    pub(super) fn encoding(self) -> u8 {
        match self {
            Self::One => 0,
            Self::Two => 1,
            Self::Four => 2,
            Self::Eight => 3,
        }
    }
}

/// `[base + index * scale + offset]`, using 64-bit wrapping address arithmetic.
/// The base and optional index are ordinary operands at every lowering stage.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct MemoryAddress<T> {
    pub base: T,
    pub index: Option<(T, Scale)>,
    pub offset: i32,
}

impl<T> MemoryAddress<T> {
    pub fn map<U>(self, mut map: impl FnMut(T) -> U) -> MemoryAddress<U> {
        MemoryAddress {
            base: map(self.base),
            index: self.index.map(|(index, scale)| (map(index), scale)),
            offset: self.offset,
        }
    }

    pub fn try_map<U, E>(
        self,
        mut map: impl FnMut(T) -> Result<U, E>,
    ) -> Result<MemoryAddress<U>, E> {
        Ok(MemoryAddress {
            base: map(self.base)?,
            index: self
                .index
                .map(|(index, scale)| map(index).map(|index| (index, scale)))
                .transpose()?,
            offset: self.offset,
        })
    }
}
