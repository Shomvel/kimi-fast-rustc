#![crate_type = "rlib"]
#![feature(core_intrinsics)]
#![allow(internal_features)]

#[inline(never)]
pub fn mul_pow2_u64(value: u64) -> u64 {
    value.wrapping_mul(4)
}

#[inline(never)]
pub fn checked_mul_pow2(value: u64) -> Option<u64> {
    value.checked_mul(4)
}

#[inline(never)]
pub fn mul_pow2_i64_sign(value: i64) -> i64 {
    value.wrapping_mul(i64::MIN)
}

#[inline(never)]
pub fn mul_pow2_u8(value: u8) -> u8 {
    value.wrapping_mul(128)
}

#[inline(never)]
pub fn mul_pow2_i16(value: i16) -> i16 {
    value.wrapping_mul(16)
}

#[inline(never)]
pub fn trunc_add(value: u64) -> u16 {
    value.wrapping_add(0xfeed_1234_5678_9abc) as u16
}

#[inline(never)]
pub fn trunc_sub(value: u32) -> u8 {
    0x807f_1234u32.wrapping_sub(value) as u8
}

#[inline(never)]
pub fn trunc_mul(value: u64) -> u32 {
    value.wrapping_mul(0x1234_5678_9abc_def1) as u32
}

#[inline(never)]
pub fn trunc_logic(value: u64) -> u16 {
    (value ^ 0xfeed_face_1234_abcd) as u16
}

#[inline(never)]
pub fn trunc_and(value: u64) -> u16 {
    (value & 0x1234_abcd_55aa_0f3c) as u16
}

#[inline(never)]
pub fn trunc_or(value: u64) -> u16 {
    (value | 0x4321_dcba_aa55_30c3) as u16
}

#[inline(never)]
pub fn trunc_extended(value: i16, other: i64) -> i16 {
    (i64::from(value)).wrapping_add(other) as i16
}

#[inline(never)]
pub fn trunc_shared(value: u64) -> (u16, u64) {
    let full = value.wrapping_add(0xfeed_1234_5678_9abc);
    (full as u16, full)
}

#[inline(never)]
pub fn opposite_left(value: u64) -> u64 {
    (value << 13) >> 13
}

#[inline(never)]
pub fn opposite_right(value: u32) -> u32 {
    (value >> 7) << 7
}

#[inline(never)]
pub fn opposite_overshift(value: u8) -> u8 {
    (value << (9u32 & 7)) >> 1
}

#[inline(never)]
pub fn opposite_shared(value: u64) -> (u64, u64) {
    let shifted = value << 13;
    (shifted >> 13, shifted)
}

#[inline(never)]
pub fn mul_shift_factor_u64(value: u64, amount: u32) -> u64 {
    value.wrapping_mul(1u64 << (amount & 63))
}

#[inline(never)]
pub fn mul_shift_factor_i16(value: i16, amount: u32) -> i16 {
    (1i16 << (amount & 15)).wrapping_mul(value)
}

#[inline(never)]
pub fn mul_shift_factor_u8(value: u8, amount: u32) -> u8 {
    value.wrapping_mul(1u8 << (amount & 7))
}

#[inline(never)]
pub fn mul_shift_factor_shared(value: u64, amount: u32) -> (u64, u64) {
    let factor = 1u64 << (amount & 63);
    (value.wrapping_mul(factor), factor)
}

/// # Safety
/// Both pointers are valid and initialized for `length` bytes, as required by
/// `compare_bytes`; their readable ranges may alias or overlap.
#[inline(never)]
pub unsafe fn memcmp_equal(left: *const u8, right: *const u8, length: usize) -> bool {
    unsafe { core::intrinsics::compare_bytes(left, right, length) == 0 }
}

/// # Safety
/// Both pointers satisfy `compare_bytes`' full readable-range contract.
#[inline(never)]
pub unsafe fn memcmp_unequal(left: *const u8, right: *const u8, length: usize) -> bool {
    unsafe { core::intrinsics::compare_bytes(left, right, length) != 0 }
}

/// # Safety
/// Both pointers are readable and initialized for at least seven bytes.
#[inline(never)]
pub unsafe fn memcmp_equal_seven(left: *const u8, right: *const u8) -> bool {
    unsafe { core::intrinsics::compare_bytes(left, right, 7) == 0 }
}

/// # Safety
/// Both pointers are readable and initialized for at least 32 bytes.
#[inline(never)]
pub unsafe fn memcmp_equal_32(left: *const u8, right: *const u8) -> bool {
    unsafe { core::intrinsics::compare_bytes(left, right, 32) == 0 }
}

/// # Safety
/// All three pointers satisfy `compare_bytes` for the complete `length`.
#[inline(never)]
pub unsafe fn memcmp_equality_phi(
    left: *const u8,
    first: *const u8,
    second: *const u8,
    length: usize,
    select_first: bool,
) -> bool {
    let comparison = if select_first {
        unsafe { core::intrinsics::compare_bytes(left, first, length) }
    } else {
        unsafe { core::intrinsics::compare_bytes(left, second, length) }
    };
    comparison == 0
}

/// # Safety
/// Both pointers satisfy `compare_bytes`' full readable-range contract.
/// The returned ordering sign must remain observable, so bcmp is ineligible.
#[inline(never)]
pub unsafe fn memcmp_order(left: *const u8, right: *const u8, length: usize) -> i32 {
    unsafe { core::intrinsics::compare_bytes(left, right, length) }
}

/// # Safety
/// Both pointers satisfy `compare_bytes`; `observed` is a disjoint writable
/// i32. Storing the comparison exposes its nonzero ordering result.
#[inline(never)]
pub unsafe fn memcmp_store_then_equal(
    left: *const u8,
    right: *const u8,
    length: usize,
    observed: &mut i32,
) -> bool {
    let comparison = unsafe { core::intrinsics::compare_bytes(left, right, length) };
    *observed = comparison;
    comparison == 0
}
