//! Scoped-condition fixtures. Every call in the differential driver must use
//! defined inputs; assert_unchecked's false inputs belong only in UB tests.

#[inline(never)]
pub fn bounds_after_guard(values: &[u8], index: usize) -> u8 {
    if index < values.len() {
        values[index]
    } else {
        0
    }
}

#[inline(never)]
pub fn repeated_bounds(values: &[u8], index: usize) -> u8 {
    values[index].wrapping_add(values[index])
}

#[inline(never)]
pub fn negated_guard(predicate: bool, value: u64) -> u64 {
    if !predicate {
        if predicate { 17 } else { value }
    } else {
        if !predicate {
            23
        } else {
            value.wrapping_add(1)
        }
    }
}

#[inline(never)]
pub unsafe fn assumed_bounds(values: &[u8; 16], index: usize) -> u8 {
    unsafe { core::hint::assert_unchecked(index < 16) };
    values[index]
}

#[inline(never)]
pub fn non_dominating(predicate: bool, choose: bool) -> u64 {
    let first = if choose {
        if predicate { 10 } else { 20 }
    } else {
        30
    };
    first + if predicate { 1 } else { 2 }
}

#[inline(never)]
pub fn changing_loop_limit(mut limit: usize, count: usize) -> usize {
    let mut result = 0;
    let mut index = 0;
    while index < count {
        if index < limit {
            result += 1;
        }
        limit = limit.saturating_sub(1);
        if index < limit {
            result += 2;
        }
        index += 1;
    }
    result
}

#[inline(never)]
fn grow(values: &mut Vec<u8>) {
    values.push(47);
}

#[inline(never)]
pub fn reloaded_length(values: &mut Vec<u8>, index: usize) -> u8 {
    let before = index < values.len();
    grow(values);
    if index < values.len() {
        values[index].wrapping_add(u8::from(before))
    } else {
        0
    }
}

#[inline(never)]
pub fn array_guard(values: &[u32; 8], index: usize) -> u32 {
    if index < 4 { values[index] } else { 0 }
}

#[inline(never)]
pub fn comparison_family(a: u64, b: u64) -> u64 {
    if a < b {
        if a <= b && a != b && b != 0 { 7 } else { 99 }
    } else {
        if a >= b { 1 } else { 99 }
    }
}

#[inline(never)]
pub fn signed_threshold(value: i64) -> u64 {
    if value < -4 {
        if value < 8 { 1 } else { 99 }
    } else {
        if value < 0 { 1 } else { 0 }
    }
}
