//! LP1: sparse direct tags across queries, transport, control flow and niches.
#[derive(Clone, Copy)]
#[repr(u64)]
pub enum Alignment {
    A0 = 1u64 << 0,
    A1 = 1u64 << 1,
    A2 = 1u64 << 2,
    A3 = 1u64 << 3,
    A4 = 1u64 << 4,
    A5 = 1u64 << 5,
    A6 = 1u64 << 6,
    A7 = 1u64 << 7,
    A8 = 1u64 << 8,
    A9 = 1u64 << 9,
    A10 = 1u64 << 10,
    A11 = 1u64 << 11,
    A12 = 1u64 << 12,
    A13 = 1u64 << 13,
    A14 = 1u64 << 14,
    A15 = 1u64 << 15,
    A16 = 1u64 << 16,
    A17 = 1u64 << 17,
    A18 = 1u64 << 18,
    A19 = 1u64 << 19,
    A20 = 1u64 << 20,
    A21 = 1u64 << 21,
    A22 = 1u64 << 22,
    A23 = 1u64 << 23,
    A24 = 1u64 << 24,
    A25 = 1u64 << 25,
    A26 = 1u64 << 26,
    A27 = 1u64 << 27,
    A28 = 1u64 << 28,
    A29 = 1u64 << 29,
    A30 = 1u64 << 30,
    A31 = 1u64 << 31,
    A32 = 1u64 << 32,
    A33 = 1u64 << 33,
    A34 = 1u64 << 34,
    A35 = 1u64 << 35,
    A36 = 1u64 << 36,
    A37 = 1u64 << 37,
    A38 = 1u64 << 38,
    A39 = 1u64 << 39,
    A40 = 1u64 << 40,
    A41 = 1u64 << 41,
    A42 = 1u64 << 42,
    A43 = 1u64 << 43,
    A44 = 1u64 << 44,
    A45 = 1u64 << 45,
    A46 = 1u64 << 46,
    A47 = 1u64 << 47,
    A48 = 1u64 << 48,
    A49 = 1u64 << 49,
    A50 = 1u64 << 50,
    A51 = 1u64 << 51,
    A52 = 1u64 << 52,
    A53 = 1u64 << 53,
    A54 = 1u64 << 54,
    A55 = 1u64 << 55,
    A56 = 1u64 << 56,
    A57 = 1u64 << 57,
    A58 = 1u64 << 58,
    A59 = 1u64 << 59,
    A60 = 1u64 << 60,
    A61 = 1u64 << 61,
    A62 = 1u64 << 62,
    A63 = 1u64 << 63,
}

#[unsafe(no_mangle)]
#[inline(never)]
pub fn lp1_alignment_as_u64(value: &Alignment) -> u64 {
    *value as u64
}

#[inline(never)]
pub fn alignment_copy(source: &Alignment, destination: &mut Alignment) {
    *destination = *source;
}

#[inline(never)]
pub fn alignment_phi(a: Alignment, b: Alignment, choose: bool) -> Alignment {
    if choose {
        a
    } else {
        b
    }
}

#[inline(never)]
pub fn optional_alignment(value: Option<Alignment>) -> u64 {
    match value {
        Some(alignment) => alignment as u64,
        None => 0,
    }
}

#[inline(never)]
pub fn alignment_loop(mut a: Alignment, b: Alignment, count: usize) -> u64 {
    for _ in 0..count {
        a = if a as u64 == 1 { b } else { Alignment::A0 };
    }
    a as u64
}

#[derive(Clone, Copy)]
#[repr(u8)]
pub enum ByteTag {
    Low = 3,
    Middle = 127,
    High = 255,
}
#[inline(never)]
pub fn byte_tag(value: &ByteTag) -> u64 {
    *value as u64
}

#[derive(Clone, Copy)]
#[repr(i8)]
pub enum SignedByteTag {
    Low = -128,
    Middle = 7,
    High = -1,
}
#[inline(never)]
pub fn signed_byte_tag(value: &SignedByteTag) -> i64 {
    *value as i64
}

#[derive(Clone, Copy)]
#[repr(i32)]
pub enum SignedTag {
    Low = -2147483648,
    Middle = 17,
    High = 2147483647,
}
#[inline(never)]
pub fn signed_tag(value: &SignedTag) -> i64 {
    *value as i64
}
#[inline(never)]
pub fn signed_phi(choose: bool) -> SignedTag {
    if choose {
        SignedTag::Low
    } else {
        SignedTag::High
    }
}

#[derive(Clone, Copy)]
#[repr(i32)]
pub enum Payload {
    Empty = -91,
    Number(u64) = 37,
    Pair(u64, u64) = 1009,
}
#[inline(never)]
pub fn payload_copy(source: &Payload, destination: &mut Payload) {
    *destination = *source;
}
#[inline(never)]
pub fn payload_phi(a: Payload, b: Payload, choose: bool) -> Payload {
    if choose {
        a
    } else {
        b
    }
}
#[inline(never)]
pub fn payload_value(value: Payload) -> u64 {
    match value {
        Payload::Empty => 91,
        Payload::Number(x) => x,
        Payload::Pair(x, y) => x ^ y,
    }
}
