//! Typed-swap snapshots and discriminants of non-enum values.

#![allow(enum_intrinsics_non_enums)]

#[repr(C)]
struct Padded {
    tag: u8,
    value: i64,
}

union Word {
    integer: u64,
    bytes: [u8; 8],
}

#[repr(i8)]
enum SignedTag {
    Negative = -7,
    Positive = 23,
}

#[inline(never)]
pub fn discriminants(value: i64) -> ([u8; 8], i8, i64, bool) {
    let scalar = std::hint::black_box(value);
    let boolean = scalar < 0;
    let tuple = (scalar, boolean);
    let structure = Padded {
        tag: 5,
        value: scalar,
    };
    let union = Word {
        integer: scalar as u64,
    };
    let text = std::hint::black_box("discriminant");
    let array = [scalar, scalar.wrapping_add(1)];
    let closure = || scalar;
    let plain = [
        std::intrinsics::discriminant_value(&scalar),
        std::intrinsics::discriminant_value(&boolean),
        std::intrinsics::discriminant_value(&tuple),
        std::intrinsics::discriminant_value(&structure),
        std::intrinsics::discriminant_value(&union),
        std::intrinsics::discriminant_value(&text),
        std::intrinsics::discriminant_value(&array),
        std::intrinsics::discriminant_value(&closure),
    ];
    let tag = if scalar < 0 {
        SignedTag::Negative
    } else {
        SignedTag::Positive
    };
    let niche = if scalar == 0 { None } else { Some(&scalar) };
    (
        plain,
        std::intrinsics::discriminant_value(&tag),
        std::intrinsics::discriminant_value(&niche) as i64,
        std::mem::discriminant(&text) == std::mem::discriminant(&"other"),
    )
}

#[inline(never)]
pub fn scalar_swap(left: i64, right: i64) -> (i64, i64, bool, bool) {
    let mut left = std::hint::black_box(left);
    let mut right = std::hint::black_box(right);
    std::mem::swap(&mut left, &mut right);
    let mut a = left < 0;
    let mut b = right < 0;
    unsafe { std::intrinsics::typed_swap_nonoverlapping(&mut a, &mut b) };
    (left, right, a, b)
}

#[inline(never)]
pub fn aggregate_swap(left: i64, right: i64) -> ([i64; 3], [i64; 3], i64, i64) {
    let mut a = [left, left.wrapping_add(3), left.wrapping_add(7)];
    let mut b = [right, right.wrapping_sub(5), right.wrapping_sub(11)];
    unsafe { std::intrinsics::typed_swap_nonoverlapping(&mut a, &mut b) };
    let mut padded_left = Padded {
        tag: 19,
        value: left,
    };
    let mut padded_right = Padded {
        tag: 29,
        value: right,
    };
    std::mem::swap(&mut padded_left, &mut padded_right);
    (
        a,
        b,
        padded_left.value.wrapping_add(padded_left.tag as i64),
        padded_right.value.wrapping_add(padded_right.tag as i64),
    )
}

#[inline(never)]
pub fn union_niche_swap(left: u64, right: u64) -> (u64, u64, bool, bool) {
    let mut a = Word {
        integer: std::hint::black_box(left),
    };
    let mut b = Word {
        integer: std::hint::black_box(right),
    };
    let mut first = Some(&left);
    let mut second = None;
    unsafe {
        std::intrinsics::typed_swap_nonoverlapping(&mut a, &mut b);
        std::intrinsics::typed_swap_nonoverlapping(&mut first, &mut second);
        // All bytes of these unions were initialized by their integer fields.
        let a = u64::from_ne_bytes(a.bytes);
        let b = u64::from_ne_bytes(b.bytes);
        (a, b, first.is_some(), second.is_some())
    }
}

struct Token {
    id: i64,
    drops: *mut i64,
}

impl Drop for Token {
    fn drop(&mut self) {
        unsafe { *self.drops += 1 };
    }
}

#[inline(never)]
pub fn owned_zst_swap(left: i64, right: i64) -> (i64, i64, i64, i64) {
    let mut drops = 0;
    let mut a = Token {
        id: left,
        drops: &mut drops,
    };
    let mut b = Token {
        id: right,
        drops: &mut drops,
    };
    std::mem::swap(&mut a, &mut b);
    let ids = (a.id, b.id);
    let before_drop = drops;
    drop(a);
    drop(b);
    let dangling = std::ptr::NonNull::<()>::dangling().as_ptr();
    unsafe { std::intrinsics::typed_swap_nonoverlapping(dangling, dangling) };
    (ids.0, ids.1, before_drop, drops)
}
