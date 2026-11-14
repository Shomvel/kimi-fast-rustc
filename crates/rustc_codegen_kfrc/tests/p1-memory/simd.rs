//! Real stdarch operations and 128-bit C ABI vector boundaries.

#![allow(improper_ctypes_definitions)]

use std::arch::x86_64::*;
use std::simd::{cmp::SimdPartialEq, Simd};

#[inline(never)]
pub unsafe extern "C" fn vector_identity(value: __m128i) -> __m128i {
    value
}

#[inline(never)]
pub unsafe extern "C" fn movemask(value: __m128i) -> i32 {
    unsafe { _mm_movemask_epi8(value) }
}

#[inline(never)]
pub unsafe extern "C" fn mixed(
    prefix: u64,
    left: __m128i,
    middle: u64,
    right: __m128i,
    suffix: u64,
) -> __m128i {
    let scalar = unsafe {
        std::mem::transmute::<[u64; 2], __m128i>([
            prefix.wrapping_add(middle.wrapping_mul(3)),
            suffix.wrapping_add(middle.wrapping_mul(5)),
        ])
    };
    unsafe { _mm_xor_si128(_mm_xor_si128(left, right), scalar) }
}

#[inline(never)]
pub unsafe extern "C" fn mixed_stack(
    a: u64,
    left: __m128i,
    b: u64,
    c: u64,
    d: u64,
    e: u64,
    right: __m128i,
    f: u64,
    g: u64,
    h: u64,
) -> __m128i {
    let first = a
        .wrapping_add(b.wrapping_mul(3))
        .wrapping_add(c.wrapping_mul(5))
        .wrapping_add(d.wrapping_mul(7));
    let second = e
        .wrapping_add(f.wrapping_mul(11))
        .wrapping_add(g.wrapping_mul(13))
        .wrapping_add(h.wrapping_mul(17));
    let scalar = unsafe { std::mem::transmute::<[u64; 2], __m128i>([first, second]) };
    unsafe { _mm_xor_si128(_mm_or_si128(left, right), scalar) }
}

#[inline(never)]
pub unsafe fn call_mixed(left: __m128i, right: __m128i, seed: u64) -> __m128i {
    unsafe {
        mixed(
            seed,
            left,
            seed.wrapping_add(7),
            right,
            seed.wrapping_add(11),
        )
    }
}

#[inline(never)]
pub unsafe fn call_callback(
    callback: unsafe extern "C" fn(u64, __m128i, u64, __m128i, u64) -> __m128i,
    left: __m128i,
    right: __m128i,
    seed: u64,
) -> __m128i {
    unsafe {
        callback(
            seed,
            left,
            seed.wrapping_add(7),
            right,
            seed.wrapping_add(11),
        )
    }
}

/// The caller supplies two readable 16-byte ranges and 128 writable bytes.
#[inline(never)]
pub unsafe fn unaligned(left: *const u8, right: *const u8, output: *mut u8) -> (i32, i32) {
    unsafe {
        let left = _mm_loadu_si128(left.cast());
        let right = _mm_loadu_si128(right.cast());
        _mm_storeu_si128(output.cast(), _mm_and_si128(left, right));
        _mm_storeu_si128(output.add(16).cast(), _mm_or_si128(left, right));
        _mm_storeu_si128(output.add(32).cast(), _mm_xor_si128(left, right));
        _mm_storeu_si128(output.add(48).cast(), _mm_andnot_si128(left, right));
        _mm_storeu_si128(output.add(64).cast(), _mm_cmpeq_epi8(left, right));
        _mm_storeu_si128(output.add(80).cast(), _mm_cmpeq_epi16(left, right));
        _mm_storeu_si128(output.add(96).cast(), _mm_cmpeq_epi32(left, right));
        _mm_storeu_si128(output.add(112).cast(), _mm_cmpgt_epi8(left, right));
        (
            _mm_movemask_epi8(left),
            _mm_movemask_epi8(_mm_cmpeq_epi8(left, right)),
        )
    }
}

#[inline(never)]
pub fn shuffle(left: [u8; 16], right: [u8; 16]) -> [u8; 16] {
    let left = Simd::<u8, 16>::from_array(left);
    let right = Simd::<u8, 16>::from_array(right);
    let shuffled: Simd<u8, 16> = unsafe {
        std::intrinsics::simd::simd_shuffle(
            left,
            right,
            const {
                Simd::<u32, 16>::from_array([
                    0, 15, 16, 31, 1, 17, 14, 30, 7, 23, 8, 24, 0, 31, 16, 15,
                ])
            },
        )
    };
    shuffled.to_array()
}

#[inline(never)]
pub fn portable_masks(bytes: [u8; 16], needle: u8) -> (u64, u64, u64) {
    let mask = Simd::from_array(bytes).simd_eq(Simd::splat(needle));
    (
        mask.to_bitmask(),
        mask.resize::<8>(false).to_bitmask(),
        mask.resize::<32>(true).to_bitmask(),
    )
}
