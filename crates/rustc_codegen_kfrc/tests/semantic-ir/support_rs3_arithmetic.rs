//! Only defined inputs are passed by the three-arm driver.
#[inline(never)]
pub unsafe fn unchecked_add_u32(a: u32, b: u32) -> u64 {
    unsafe { a.unchecked_add(b) as u64 }
}

#[inline(never)]
pub unsafe fn unchecked_sub_i32(a: i32, b: i32) -> i64 {
    unsafe { a.unchecked_sub(b) as i64 }
}

#[inline(never)]
pub unsafe fn unchecked_mul_i32(a: i32, b: i32) -> i64 {
    unsafe { a.unchecked_mul(b) as i64 }
}

#[inline(never)]
pub unsafe fn unchecked_shifts(a: u32, signed: i32, amount: u32) -> (u32, u32, i32) {
    unsafe {
        (
            a.unchecked_shl(amount),
            a.unchecked_shr(amount),
            signed.unchecked_shr(amount),
        )
    }
}

#[inline(never)]
pub fn wrapping_add_u32(a: u32, b: u32) -> u64 {
    a.wrapping_add(b) as u64
}

#[inline(never)]
pub fn checked_add_u32(a: u32, b: u32) -> Option<u32> {
    a.checked_add(b)
}

#[inline(never)]
pub unsafe fn distance24_unsigned(a: *const [u8; 24], b: *const [u8; 24]) -> usize {
    unsafe { a.offset_from_unsigned(b) }
}

#[inline(never)]
pub unsafe fn distance24_signed(a: *const [u8; 24], b: *const [u8; 24]) -> isize {
    unsafe { a.offset_from(b) }
}

#[inline(never)]
pub fn iterator_length24(values: &[[u8; 24]]) -> usize {
    values.iter().len()
}
