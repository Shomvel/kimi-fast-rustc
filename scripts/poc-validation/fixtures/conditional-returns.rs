#![crate_type = "rlib"]

unsafe extern "Rust" {
    fn conditional_observe(value: u64) -> u64;
}

#[inline(never)]
fn equal_bytes(left: &[u8], right: &[u8]) -> bool {
    left == right
}

#[inline(never)]
fn guarded_equal(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        false
    } else {
        equal_bytes(left, right)
    }
}

#[inline(never)]
pub fn classify(input: &[u8]) -> u64 {
    if guarded_equal(input, b"ns") {
        1
    } else if guarded_equal(input, b"microseconds") {
        2
    } else if guarded_equal(input, b"ms") {
        3
    } else if guarded_equal(input, b"second") {
        4
    } else if guarded_equal(input, b"minute") {
        5
    } else if guarded_equal(input, b"hour") {
        6
    } else if guarded_equal(input, b"day") {
        7
    } else if guarded_equal(input, b"week") {
        8
    } else {
        0
    }
}

#[inline(never)]
fn equal_with_observation(left: usize, right: usize) -> bool {
    if left != right {
        false
    } else {
        unsafe { conditional_observe(left as u64) != 0 }
    }
}

#[inline(never)]
fn observe_before_guard(left: usize, right: usize) -> bool {
    let observed = unsafe { conditional_observe(left as u64) };
    if left != right { false } else { observed != 0 }
}

#[inline(never)]
pub fn guarded_observation(value: usize) -> bool {
    equal_with_observation(value, 7)
}

#[inline(never)]
pub fn required_observation(value: usize) -> bool {
    observe_before_guard(value, 7)
}

#[inline(never)]
unsafe fn raw_guard(left: *const u8, left_len: usize, right: *const u8, right_len: usize) -> bool {
    if left_len != right_len {
        false
    } else if left_len == 0 {
        true
    } else {
        unsafe { *left == *right }
    }
}

/// # Safety
/// When `length == 17`, `pointer` and address 1 must each be readable for one
/// byte. The native fixture uses only unequal lengths, so neither is read.
#[inline(never)]
pub unsafe fn unequal_lengths_do_not_read(pointer: *const u8, length: usize) -> bool {
    unsafe { raw_guard(pointer, length, 1usize as *const u8, 17) }
}

#[inline(never)]
pub fn metadata_roundtrip(input: &str) -> usize {
    // Both conversions preserve this valid string's complete pointer pair.
    let bytes: &[u8] = unsafe { core::mem::transmute(input) };
    let text: &str = unsafe { core::mem::transmute(bytes) };
    text.len() ^ bytes.len().rotate_left(7)
}

/// # Safety
/// `pointer` is initialized, aligned and writable for an `&str`; `next` and
/// the previous string refer to live data for the duration of the call.
#[inline(never)]
pub unsafe fn metadata_from_snapshot<'a>(pointer: *mut &'a str, next: &'a str) -> (usize, usize) {
    unsafe {
        let snapshot = *pointer;
        *pointer = next;
        let bytes: &[u8] = core::mem::transmute(snapshot);
        let reloaded = *pointer;
        (bytes.len(), reloaded.len())
    }
}
