//! Defined Rust fixtures for the later IV/BCE consumers of LoopInfo.
//! Read-only loop analysis alone does not promise fewer bounds checks.

#[inline(never)]
pub fn while_sum(values: &[u32]) -> u64 {
    let mut index = 0;
    let mut sum = 0u64;
    while index < values.len() {
        sum = sum.wrapping_add(values[index] as u64);
        index += 1;
    }
    sum
}

#[inline(never)]
pub fn for_sum(values: &[u32]) -> u64 {
    let mut sum = 0u64;
    for index in 0..values.len() {
        sum = sum.wrapping_add(values[index] as u64);
    }
    sum
}

#[inline(never)]
pub fn enumerate_sum(values: &[u32]) -> u64 {
    let mut sum = 0u64;
    for (index, &value) in values.iter().enumerate() {
        sum = sum.wrapping_add((index as u64) ^ (value as u64));
    }
    sum
}

#[inline(never)]
pub fn chunks_sum(values: &[u32], size: usize) -> u64 {
    let mut sum = 0u64;
    for chunk in values.chunks(size) {
        sum = sum.wrapping_add(chunk[0] as u64);
    }
    sum
}

#[inline(never)]
pub fn nested_break_continue(values: &[u32], rounds: usize) -> u64 {
    let mut sum = 0u64;
    for round in 0..rounds {
        let mut index = 0;
        while index < values.len() {
            let value = values[index];
            index += 1;
            if value == 0 {
                continue;
            }
            if value == u32::MAX {
                break;
            }
            sum = sum.wrapping_add((value as u64) ^ (round as u64));
        }
    }
    sum
}

#[inline(never)]
pub fn length_after_push(values: &mut Vec<u32>, value: u32) -> u32 {
    let index = values.len();
    values.push(value);
    // The old index equals the old length but is in range for the new length.
    values[index]
}
