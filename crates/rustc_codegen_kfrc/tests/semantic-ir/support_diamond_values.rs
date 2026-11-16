#[inline(never)]
pub fn increment_or_zero(value: u8) -> u8 {
    value.checked_add(1).unwrap_or(0)
}

#[inline(never)]
pub fn decrement_or_max(value: u8) -> u8 {
    value.checked_sub(1).unwrap_or(u8::MAX)
}

#[inline(never)]
pub fn equivalent_constant(value: u8) -> u8 {
    if value == 37 {
        112
    } else {
        value.wrapping_mul(3).wrapping_add(1)
    }
}

#[inline(never)]
pub fn different_constant(value: u8) -> u8 {
    if value == 37 {
        113
    } else {
        value.wrapping_mul(3).wrapping_add(1)
    }
}

#[inline(never)]
pub fn nonunit_increment(value: u8) -> u8 {
    value.checked_add(2).unwrap_or(0)
}

#[inline(never)]
pub fn observed_increment(value: u8, trace: &mut u64) -> u8 {
    if value == u8::MAX {
        *trace = trace.wrapping_add(7);
        0
    } else {
        value.wrapping_add(1)
    }
}

#[inline(never)]
pub fn word_increment(value: u64) -> u64 {
    value.checked_add(1).unwrap_or(0)
}

#[inline(never)]
pub fn signed_equality(value: i64) -> i64 {
    if value == i64::MAX {
        i64::MIN
    } else {
        value.wrapping_add(1)
    }
}

#[inline(never)]
pub fn signed_guarded_unchecked(value: i64) -> i64 {
    if value == i64::MAX {
        i64::MIN
    } else {
        // SAFETY: The only signed overflow input for adding one was excluded.
        unsafe { value.unchecked_add(1) }
    }
}
