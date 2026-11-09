//! Safe callers exercise initialized values and opaque, uninitialized storage.
//! Only the intended typed boundary assumes initialization.
use std::mem::MaybeUninit;

#[repr(C)]
struct Pair {
    small: u8,
    large: u64,
}

#[inline(never)]
pub fn initialized_roundtrip(value: u32) -> u32 {
    let stored = MaybeUninit::new(value);
    // SAFETY: new initialized the complete u32.
    unsafe { stored.assume_init() }
}

#[inline(never)]
pub fn opaque_uninitialized() -> MaybeUninit<[u64; 4]> {
    MaybeUninit::uninit()
}

#[inline(never)]
pub fn invalid_bool_storage() -> u8 {
    let mut stored = MaybeUninit::<bool>::uninit();
    // SAFETY: access only the one-byte representation. No bool is created.
    unsafe {
        stored.as_mut_ptr().cast::<u8>().write(2);
        stored.as_ptr().cast::<u8>().read()
    }
}

#[inline(never)]
pub fn partial_initialization(small: u8, large: u64) -> u64 {
    let mut stored = MaybeUninit::<Pair>::uninit();
    let address = stored.as_mut_ptr();
    // SAFETY: both fields are initialized before assume_init. Padding remains
    // unspecified and must not become an observable poison field during copy.
    unsafe {
        std::ptr::addr_of_mut!((*address).small).write(small);
        std::ptr::addr_of_mut!((*address).large).write(large);
        let value = stored.assume_init();
        u64::from(value.small) ^ value.large
    }
}

#[inline(never)]
pub fn array_iterator(mut values: [u64; 4]) -> u64 {
    values[1] ^= values[3];
    let mut iterator = values.into_iter();
    let first = iterator.next().unwrap();
    let last = iterator.next_back().unwrap();
    let mut copied = iterator.clone();
    let mut result = first ^ last;
    while let Some(value) = iterator.next() {
        result = result.wrapping_add(value);
    }
    while let Some(value) = copied.next_back() {
        result ^= value;
    }
    result
}

#[inline(never)]
pub fn empty_and_singleton(value: u64) -> u64 {
    let mut empty = [0u64; 0].into_iter();
    let mut one = [value].into_iter();
    if empty.next().is_some() || empty.next_back().is_some() {
        return 999;
    }
    let result = one.next_back().unwrap();
    if one.next().is_some() {
        888
    } else {
        result
    }
}
