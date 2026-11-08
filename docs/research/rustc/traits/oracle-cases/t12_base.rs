pub trait Tr { type A; fn get(&self) -> Self::A; }
impl Tr for u32 { type A = u64; fn get(&self) -> u64 { *self as u64 } }
impl Tr for bool { type A = u8; fn get(&self) -> u8 { *self as u8 } }
pub fn via_param<T: Tr>(t: T) -> T::A { t.get() }
pub fn via_eq<T: Tr<A = u64>>(t: T) -> u64 { via_param(t) }
