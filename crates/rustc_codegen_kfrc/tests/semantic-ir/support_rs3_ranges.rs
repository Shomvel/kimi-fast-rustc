//! Defined-input RS3.2 fixtures. Invalid scalar representations belong only in
//! interpreter tests, never in the LLVM/native differential driver.
use core::{mem::MaybeUninit, num::NonZeroU32, ptr::NonNull};

#[inline(never)]
pub fn nonzero(value: NonZeroU32) -> bool {
    value.get() != 0
}

#[inline(never)]
pub fn character(value: char) -> bool {
    (value as u32) <= 0x10ffff
}

#[inline(never)]
pub fn boolean(value: bool) -> usize {
    value as usize
}

#[inline(never)]
pub fn nonnull(value: NonNull<u8>) -> bool {
    value.as_ptr().is_null()
}

#[inline(never)]
pub fn raw_maybe_uninit(bits: u32) -> bool {
    let value = MaybeUninit::<NonZeroU32>::zeroed();
    let mut value = value;
    unsafe {
        value.as_mut_ptr().cast::<u32>().write(bits);
        value.as_ptr().cast::<u32>().read() != 0
    }
}

// FnPtr currently shares an IR scalar identity with *const (). Both null raw
// pointers and callable function pointers in these signatures are valid Rust.
#[inline(never)]
pub fn callback_and_nullable_raw(callback: fn(u32) -> u32, raw: *const (), value: u32) -> u32 {
    callback(value).wrapping_add(if raw == 0usize as *const () { 17 } else { 29 })
}

#[inline(never)]
pub fn callback_tuple_and_nullable_raw(
    callback: (fn(u32) -> u32, usize),
    raw: (*const (), usize),
    value: u32,
) -> u32 {
    (callback.0)(value)
        .wrapping_add((callback.1 ^ raw.1) as u32)
        .wrapping_add(if raw.0 == 0usize as *const () { 17 } else { 29 })
}

#[inline(never)]
pub fn callback_array_and_nullable_raw(
    callback: [fn(u32) -> u32; 1],
    raw: [*const (); 1],
    value: u32,
) -> u32 {
    callback[0](value).wrapping_add(if raw[0] == 0usize as *const () {
        17
    } else {
        29
    })
}
