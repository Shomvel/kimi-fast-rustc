//! 128-bit bit operations remain an explicit unsupported boundary.
#[inline(never)]
pub fn count_128(value: u128) -> u32 {
    value.count_ones()
}
