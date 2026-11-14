//! Byte transport across locals, copies, variant payloads and ABI boundaries.
use std::mem::MaybeUninit;

#[inline(never)]
pub fn mixed_width(mut word: u64, replacement: u32, byte: u8) -> u64 {
    let address = &mut word as *mut u64 as *mut u8;
    // Every byte is initialized by `word`; these overlapping writes replace
    // exactly four bytes, then one byte of that replacement.
    unsafe {
        address.add(2).cast::<u32>().write_unaligned(replacement);
        address.add(5).write(byte);
    }
    word
}

#[inline(never)]
pub fn partial_copy(mut words: [u64; 2], source: &[u8; 7]) -> [u64; 2] {
    // Both allocations are initialized and disjoint. The range crosses the
    // word boundary; bytes outside it keep their original values.
    unsafe {
        std::ptr::copy_nonoverlapping(source.as_ptr(), words.as_mut_ptr().cast::<u8>().add(3), 7);
    }
    words
}

#[inline(never)]
pub fn overlapping_copy(a: u64, b: u64, reverse: bool) -> [u64; 2] {
    let mut words = [a, b];
    let address = words.as_mut_ptr().cast::<u8>();
    // Both ranges are in the initialized 16-byte object. `copy` permits
    // overlap and must snapshot all eleven source bytes before any write.
    unsafe {
        if reverse {
            std::ptr::copy(address.add(3), address, 11);
        } else {
            std::ptr::copy(address, address.add(3), 11);
        }
    }
    words
}

#[inline(never)]
pub fn raw_snapshot(word: u64) -> u64 {
    let mut storage = MaybeUninit::<[u64; 2]>::uninit();
    // The second word stays uninitialized across the copy. Only the first
    // initialized word is read as a Rust value.
    unsafe {
        storage.as_mut_ptr().cast::<u64>().write(word);
        let snapshot = storage;
        snapshot.as_ptr().cast::<u64>().read()
    }
}

#[inline(never)]
pub fn make_result(seed: u64, fail: bool) -> Result<[u64; 3], (u64, u8)> {
    if fail {
        Err((seed, (seed >> 8) as u8))
    } else {
        Ok([seed, seed.wrapping_add(1), seed.wrapping_mul(7)])
    }
}

#[inline(never)]
pub fn forward_result(seed: u64, fail: bool) -> Result<[u64; 3], (u64, u8)> {
    make_result(seed, fail)
}

#[inline(never)]
pub fn forward_parameter(value: Result<[u64; 3], (u64, u8)>) -> Result<[u64; 3], (u64, u8)> {
    value
}

#[inline(never)]
pub fn forward_raw(value: MaybeUninit<[u64; 3]>) -> MaybeUninit<[u64; 3]> {
    value
}
