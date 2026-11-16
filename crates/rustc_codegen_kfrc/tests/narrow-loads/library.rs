//! Shared loads and nearby high-bit counterexamples through real rustc.
#[inline(never)]
pub fn byte(value: &u8, saved: &mut u8) -> u64 {
    let value = *value;
    *saved = value;
    u64::from(value)
}

#[inline(never)]
pub fn half(value: &u16, saved: &mut u16) -> u64 {
    let value = *value;
    *saved = value;
    u64::from(value)
}

#[inline(never)]
pub fn word(value: &u32, saved: &mut u32) -> u64 {
    let value = *value;
    *saved = value;
    u64::from(value)
}

#[inline(never)]
pub fn signed(value: &i8, saved: &mut i16) -> u64 {
    let value = i16::from(*value);
    *saved = value;
    u64::from(value as u16)
}

#[inline(never)]
pub fn increment_byte(value: &u8, saved: &mut u8) -> u64 {
    let value = value.wrapping_add(1);
    *saved = value;
    u64::from(value)
}

#[inline(never)]
pub fn increment_half(value: &u16, saved: &mut u16) -> u64 {
    let value = value.wrapping_add(1);
    *saved = value;
    u64::from(value)
}

#[inline(never)]
pub fn parameter(value: u8) -> u64 {
    u64::from(value.wrapping_add(1))
}
