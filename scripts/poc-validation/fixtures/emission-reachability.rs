//! Compile this library with each backend and link the same external driver.
//! Fully inlined private helpers may disappear; address-taken static targets
//! and recursive calls must remain available in the emitted object.

#[inline(always)]
fn embedded(value: u64) -> u64 {
    value ^ 0x5a
}

#[inline(never)]
fn callback(value: u64) -> u64 {
    value.wrapping_mul(3).wrapping_add(7)
}

static CALLBACKS: [fn(u64) -> u64; 1] = [callback];

#[inline(never)]
fn recursive(value: u64) -> u64 {
    if value == 0 { 1 } else { value.wrapping_add(recursive(value - 1)) }
}

#[inline(never)]
pub fn evaluate(value: u64) -> u64 {
    embedded(value) ^ CALLBACKS[0](value) ^ recursive(value % 9)
}
