//! Defined-input fixtures for validity-range consumers: niche tags compared
//! against their legal encodings, Boolean branches and width canonicalization.
//! Invalid representations belong only in interpreter tests.
use core::cmp::Ordering;

/// `B` is the untagged variant inside rustc's niche range `0..=2`; its
/// relative niche slot is never a valid tag.
#[derive(Clone, Copy)]
pub enum MiddleUntagged {
    A,
    B(bool),
    C,
}

/// Valid tags 2..=254: the outer niche wraps past 255 (255, 0, 1).
#[repr(u8)]
#[derive(Clone, Copy)]
pub enum Sparse {
    Low = 2,
    High = 254,
}

#[derive(Clone, Copy)]
pub enum Wrapped {
    Value(Sparse),
    Y,
    Z,
    W,
}

#[inline(never)]
pub fn make_middle(selector: u8) -> MiddleUntagged {
    match selector % 4 {
        0 => MiddleUntagged::A,
        1 => MiddleUntagged::B(true),
        2 => MiddleUntagged::B(false),
        _ => MiddleUntagged::C,
    }
}

#[inline(never)]
pub fn classify_middle(value: MiddleUntagged) -> u32 {
    match value {
        MiddleUntagged::A => 10,
        MiddleUntagged::B(flag) => 20 + u32::from(flag),
        MiddleUntagged::C => 30,
    }
}

#[inline(never)]
pub fn classify_middle_ref(value: &MiddleUntagged) -> u32 {
    if let MiddleUntagged::B(flag) = value {
        return 40 + u32::from(*flag);
    }
    if matches!(value, MiddleUntagged::C) { 50 } else { 60 }
}

#[inline(never)]
pub fn classify_optional_middle(value: Option<MiddleUntagged>) -> u32 {
    match value {
        None => 0,
        Some(inner) => classify_middle(inner),
    }
}

#[inline(never)]
pub fn make_wrapped(selector: u8) -> Wrapped {
    match selector % 5 {
        0 => Wrapped::Value(Sparse::Low),
        1 => Wrapped::Value(Sparse::High),
        2 => Wrapped::Y,
        3 => Wrapped::Z,
        _ => Wrapped::W,
    }
}

#[inline(never)]
pub fn classify_wrapped(value: Wrapped) -> u32 {
    match value {
        Wrapped::Value(Sparse::Low) => 1,
        Wrapped::Value(Sparse::High) => 2,
        Wrapped::Y => 3,
        Wrapped::Z => 4,
        Wrapped::W => 5,
    }
}

#[inline(never)]
pub fn classify_wrapped_ref(value: &Wrapped) -> u32 {
    if matches!(value, Wrapped::Z) {
        return 7;
    }
    if let Wrapped::Value(inner) = value {
        return *inner as u32;
    }
    9
}

#[inline(never)]
pub fn nested_option(value: Option<Option<bool>>) -> u32 {
    match value {
        None => 0,
        Some(None) => 1,
        Some(Some(false)) => 2,
        Some(Some(true)) => 3,
    }
}

#[inline(never)]
pub fn optional_ordering(left: u32, right: Option<u32>) -> i32 {
    match right.map(|right| left.cmp(&right)) {
        None => 9,
        Some(Ordering::Less) => -1,
        Some(Ordering::Equal) => 0,
        Some(Ordering::Greater) => 1,
    }
}

/// The callback is compiled by LLVM: only the low byte of its `bool` result
/// is defined by the ABI.
#[inline(never)]
pub fn branch_on_negated_call(callback: fn(u32) -> bool, value: u32) -> u32 {
    if !callback(value) { value.wrapping_add(3) } else { value ^ 5 }
}

#[inline(never)]
pub fn branch_on_double_negation(callback: fn(u32) -> bool, value: u32) -> u32 {
    let flag = callback(value);
    if !!flag { 11 } else { 13 }
}

#[inline(never)]
pub fn negated_flag_value(callback: fn(u32) -> bool, value: u32) -> (bool, u8) {
    let flag = !callback(value);
    (flag, flag as u8)
}

#[inline(never)]
pub fn bool_casts(flag: bool) -> (i8, u32, i64) {
    (flag as i8, flag as u32, flag as i64)
}

#[inline(never)]
pub fn widening_casts(byte: u8, signed: i8, half: u16, character: char) -> (u32, i32, i64, u64, u32) {
    (
        byte as u32,
        byte as i32,
        signed as i64,
        half as u64,
        character as u32,
    )
}

#[inline(never)]
pub fn narrowing_casts(word: u32, signed: i32) -> (u8, u8, i8, u16, i8) {
    (
        (word & 0x3f) as u8,
        word as u8,
        signed as i8,
        (word >> 20) as u16,
        ((signed >> 28) & 0x7) as i8,
    )
}

#[inline(never)]
pub fn bitwise_narrow(a: u8, b: u8, c: i8, d: i8) -> (u8, u8, u8, i8, i8, i8) {
    (a & b, a | b, a ^ b, c & d, c | d, c ^ d)
}

#[inline(never)]
pub fn shifts_narrow(a: u16, b: i16, amount: u32) -> (u16, i16, u8) {
    (
        a.wrapping_shr(amount),
        b.wrapping_shr(amount),
        (a as u8).wrapping_shr(amount),
    )
}

#[inline(never)]
pub fn division_narrow(a: u8, b: u8, c: i16, d: i16) -> (u8, u8, i16) {
    let b = b.max(1);
    let d = if d == 0 || (c == i16::MIN && d == -1) { 7 } else { d };
    (a / b, a % b, c % d)
}

/// The byte read through a raw pointer from `MaybeUninit<bool>` storage is a
/// plain `u8`: no Boolean range applies to it.
#[inline(never)]
pub fn raw_byte_branch(storage: &core::mem::MaybeUninit<u8>) -> u32 {
    let byte = unsafe { storage.as_ptr().read() };
    if byte ^ 1 != 0 { u32::from(byte) + 100 } else { 7 }
}
