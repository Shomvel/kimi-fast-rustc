#![crate_type = "rlib"]

#[inline(never)]
pub fn unsigned_chain(value: u64) -> u16 {
    ((value as u32) as u64) as u16
}

#[inline(never)]
pub fn signed_chain(value: i8) -> i64 {
    ((value as u16) as i16) as i64
}

#[inline(never)]
pub fn sign_after_truncation(value: u64) -> i64 {
    (value as i16) as i64
}

#[inline(never)]
pub fn demanded_low(value: u64) -> u64 {
    let a = (value | 0xfeed_0000) as u8;
    let b = (value & 0x0000_ffff) as u8;
    let c = (value ^ 0xabcd_0000) as u8;
    u64::from(a)
        .wrapping_add(u64::from(b))
        .wrapping_add(u64::from(c))
}

#[inline(never)]
pub fn demanded_shared(value: u64) -> u64 {
    let shared = value | 0xabcd_0000;
    let low = shared as u16;
    let high = (shared >> 16) as u16;
    u64::from(low) | (u64::from(high) << 16)
}

#[inline(never)]
pub fn shifted_low(value: i64) -> u16 {
    ((value >> 11) as u16) & 0x001f
}

#[inline(never)]
pub fn known_sign_bits(value: i8) -> i8 {
    ((value | i8::MIN) >> 3) | -32
}

#[inline(never)]
pub fn scalar_validity(boolean: bool, character: char) -> u64 {
    u64::from((boolean as u8) as u16) ^ u64::from(character as u32)
}

#[inline(never)]
pub fn fixed_array_snapshot(a: u64, b: u64) -> u64 {
    let mut values = [a, b, a ^ b, a.wrapping_add(b)];
    let old = values[1];
    values[1] = a;
    values[2] = old;
    old ^ values[1] ^ values[2] ^ values[3]
}

#[inline(never)]
pub fn nested_fixed_array(a: u64, b: u64) -> u64 {
    let mut values = [[a, b], [b, a]];
    let old = values[1][0];
    values[1][0] = a.wrapping_add(b);
    values[0][1] = old;
    values[0][1] ^ values[1][0]
}

#[inline(never)]
pub fn fixed_count_recurrence(a: u64, b: u64) -> u64 {
    let mut left = a;
    let mut right = b;
    let mut count = 0u64;
    while count < 4 {
        let next = left.wrapping_add(right);
        left = (right << 7) ^ count;
        right = next;
        count = count.wrapping_add(1);
    }
    left ^ right
}

#[inline(never)]
pub fn signed_countdown(value: u64) -> u64 {
    let mut result = value;
    let mut count = 2i8;
    while count > -2 {
        result = result.wrapping_add(count as i64 as u64);
        count = count.wrapping_sub(1);
    }
    result
}

#[inline(never)]
pub fn wrapping_byte_count(value: u64) -> u64 {
    let mut result = value;
    let mut count = 254u8;
    while count != 2 {
        result = result.wrapping_add(u64::from(count));
        count = count.wrapping_add(1);
    }
    result
}

#[inline(never)]
pub fn partial_redundancy(a: u64, b: u64, condition: bool) -> u64 {
    let selected;
    let observed;
    if condition {
        selected = a;
        observed = std::hint::black_box(a.wrapping_mul(7));
    } else {
        selected = b;
        observed = 0;
    }
    selected.wrapping_mul(7).wrapping_add(observed)
}

#[inline(never)]
pub fn two_phi_redundancy(a: u64, b: u64, condition: bool) -> u64 {
    let left;
    let right;
    let observed;
    if condition {
        left = a;
        right = b;
        observed = std::hint::black_box(a.wrapping_add(b));
    } else {
        left = b;
        right = a ^ 9;
        observed = 0;
    }
    left.wrapping_add(right).wrapping_add(observed)
}
