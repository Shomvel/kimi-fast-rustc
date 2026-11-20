#[inline(never)]
pub fn ascii_range(value: u8) -> bool {
    value >= b'A' && value <= b'Z'
}

#[inline(never)]
pub fn ascii_outside(value: u8) -> bool {
    value < b'A' || value > b'Z'
}

#[inline(never)]
pub fn ascii_letters(value: u8) -> bool {
    (value >= b'A' && value <= b'Z') || (value >= b'a' && value <= b'z')
}

#[inline(never)]
pub fn signed_range(value: i8) -> bool {
    value > -65 && value < 65
}

#[inline(never)]
pub fn signed_word_range(value: i64) -> bool {
    value >= -37 && value <= 91
}

#[inline(never)]
pub fn high_word_range(value: u64) -> bool {
    value >= u64::MAX - 500 && value <= u64::MAX - 5
}

#[inline(never)]
pub fn different_failure_values(value: u64) -> u64 {
    if value < 17 {
        1
    } else if value > 23 {
        2
    } else {
        0
    }
}

#[inline(never)]
pub fn effect_between_bounds(value: u64, trace: &mut u64) -> bool {
    if value >= 17 {
        *trace = trace.wrapping_add(value);
        value <= 23
    } else {
        false
    }
}

#[inline(never)]
pub fn loop_ranges(values: &[u8]) -> usize {
    let mut count = 0;
    for &value in values {
        if value >= b'A' && value <= b'Z' {
            count += 1;
        }
        if value as i8 >= -7 && value as i8 <= 11 {
            count += 3;
        }
    }
    count
}

#[inline(never)]
pub fn character_digits(value: char) -> bool {
    value >= '0' && value <= '9'
}

#[inline(never)]
pub fn character_letters(value: char) -> bool {
    (value >= 'A' && value <= 'Z') || (value >= 'a' && value <= 'z')
}

#[inline(never)]
pub fn character_scalar_edges(value: char) -> bool {
    (value >= '\u{d7ff}' && value <= '\u{e000}') || (value >= '\u{10fffe}' && value <= '\u{10ffff}')
}

#[inline(never)]
pub fn character_effect_between_bounds(value: char, trace: &mut u64) -> bool {
    if value >= 'A' {
        *trace = trace.wrapping_add(value as u64);
        value <= 'Z'
    } else {
        false
    }
}
