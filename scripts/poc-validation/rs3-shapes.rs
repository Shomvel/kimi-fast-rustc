//! Public machine-shape probes for RS3, using stable Rust 1.94.1 APIs.
//!
//! Compile the same source with LLVM and kfrc, keeping overflow-checks=off,
//! debug-assertions=off and panic=abort explicit in the recorded command.
//! no_mangle names identify exact object symbols; inline(never) preserves probes.
//! These are shape fixtures, not a runtime benchmark. Count instructions and
//! panic relocations per symbol, and retain the corresponding IR/graph evidence.
//!
//! Unsafe cases must only be executed with their documented preconditions.
//! An invalid input is not a differential-test case: it invokes Rust UB.

// NUW removes the result's wrapping canonicalization, not ABI input cleanup.
// In particular, kfrc may still mask incoming u32 arguments independently.
#[unsafe(no_mangle)]
#[inline(never)]
pub unsafe fn ua(a: u32, b: u32) -> u64 {
    // Safety: caller guarantees the mathematical sum fits in u32.
    unsafe { a.unchecked_add(b) as u64 }
}

// The unchecked amount is in range; an output-width mask is a separate issue.
#[unsafe(no_mangle)]
#[inline(never)]
pub unsafe fn ushl(x: u32, shift: u32) -> u32 {
    // Safety: caller guarantees shift < 32.
    unsafe { x.unchecked_shl(shift) }
}

// Element distance is exact: x86 should need no div/idiv for the factor 24.
#[unsafe(no_mangle)]
#[inline(never)]
pub unsafe fn ofs24(a: *const [u8; 24], b: *const [u8; 24]) -> usize {
    // Safety: pointers delimit elements of the same allocation, a >= b, and
    // their byte distance is a multiple of 24 and representable by isize.
    unsafe { a.offset_from_unsigned(b) }
}

// Signed offset admits negative distances; pointer subtraction must not gain NSW.
#[unsafe(no_mangle)]
#[inline(never)]
pub unsafe fn ofs24_signed(a: *const [u8; 24], b: *const [u8; 24]) -> isize {
    // Safety: same allocation, integral element distance, within isize bounds.
    unsafe { a.offset_from(b) }
}

#[unsafe(no_mangle)]
#[inline(never)]
pub fn ilen(values: &[[u8; 24]]) -> usize {
    values.iter().len()
}

#[unsafe(no_mangle)]
#[inline(never)]
pub fn nz(value: core::num::NonZeroU32) -> bool {
    value.get() != 0
}

#[unsafe(no_mangle)]
#[inline(never)]
pub fn ch(value: char) -> bool {
    (value as u32) <= 0x10ffff
}

#[unsafe(no_mangle)]
#[inline(never)]
pub fn bl(value: bool) -> usize {
    value as usize
}

// The user's guard dominates indexing: no separate panic-bounds edge is needed.
#[unsafe(no_mangle)]
#[inline(never)]
pub fn bc(values: &[u8], index: usize) -> u8 {
    if index < values.len() {
        values[index]
    } else {
        0
    }
}

// One necessary bounds check remains; the second identical check is redundant.
#[unsafe(no_mangle)]
#[inline(never)]
pub fn bc2(values: &[u8], index: usize) -> u8 {
    values[index].wrapping_add(values[index])
}

// Requires implication (index < 4 implies index < 8), not identical predicates.
#[unsafe(no_mangle)]
#[inline(never)]
pub fn arr(values: &[u32; 8], index: usize) -> u32 {
    if index < 4 { values[index] } else { 0 }
}

#[unsafe(no_mangle)]
#[inline(never)]
pub unsafe fn asm(index: usize, values: &[u8; 16]) -> u8 {
    // Safety: caller guarantees index < 16.
    unsafe { core::hint::assert_unchecked(index < 16) };
    values[index]
}

// Wrapping and ordinary unchecked-build arithmetic must not inherit NUW/NSW.
// These probes deliberately permit overflow and shift amounts >= the width.
#[unsafe(no_mangle)]
#[inline(never)]
pub fn wrapping_add_widen(a: u32, b: u32) -> u64 {
    a.wrapping_add(b) as u64
}

#[unsafe(no_mangle)]
#[inline(never)]
pub fn plain_add_widen(a: u32, b: u32) -> u64 {
    (a + b) as u64
}

#[unsafe(no_mangle)]
#[inline(never)]
pub fn wrapping_shl(x: u32, shift: u32) -> u32 {
    x.wrapping_shl(shift)
}

#[unsafe(no_mangle)]
#[inline(never)]
pub fn checked_add(a: u32, b: u32) -> Option<u32> {
    a.checked_add(b)
}
