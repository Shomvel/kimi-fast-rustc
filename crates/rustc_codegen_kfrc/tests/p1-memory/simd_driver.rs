//! LLVM-side byte-array references for the kfrc SIMD implementation and ABI.

#![allow(improper_ctypes_definitions)]

use std::arch::x86_64::__m128i;

fn vector(bytes: [u8; 16]) -> __m128i {
    unsafe { std::mem::transmute(bytes) }
}

fn bytes(vector: __m128i) -> [u8; 16] {
    unsafe { std::mem::transmute(vector) }
}

fn scalar_bytes(first: u64, second: u64) -> [u8; 16] {
    let mut result = [0; 16];
    result[..8].copy_from_slice(&first.to_le_bytes());
    result[8..].copy_from_slice(&second.to_le_bytes());
    result
}

fn mixed_expected(
    prefix: u64,
    left: [u8; 16],
    middle: u64,
    right: [u8; 16],
    suffix: u64,
) -> [u8; 16] {
    let scalar = scalar_bytes(
        prefix.wrapping_add(middle.wrapping_mul(3)),
        suffix.wrapping_add(middle.wrapping_mul(5)),
    );
    std::array::from_fn(|index| left[index] ^ right[index] ^ scalar[index])
}

#[inline(never)]
unsafe extern "C" fn callback(
    prefix: u64,
    left: __m128i,
    middle: u64,
    right: __m128i,
    suffix: u64,
) -> __m128i {
    vector(mixed_expected(
        prefix,
        bytes(left),
        middle,
        bytes(right),
        suffix,
    ))
}

pub fn run() {
    let cases = [
        ([0; 16], [0; 16]),
        ([0xff; 16], [0; 16]),
        ([0x80; 16], [0x7f; 16]),
        (
            std::array::from_fn(|index| index as u8),
            std::array::from_fn(|index| (index + 16) as u8),
        ),
        (
            [
                0, 0x7f, 0x80, 0xff, 1, 3, 5, 7, 0xfe, 0x81, 0x40, 0xc0, 17, 19, 23, 29,
            ],
            [
                0, 0x7f, 0x80, 0xff, 1, 9, 5, 11, 0xff, 0x81, 0x40, 0xc0, 17, 19, 0, 29,
            ],
        ),
    ];
    for (case, (left, right)) in cases.into_iter().enumerate() {
        assert_eq!(
            unsafe { bytes(memory::simd::vector_identity(vector(left))) },
            left
        );
        let mut left_storage = [0xaa; 20];
        let mut right_storage = [0xbb; 22];
        left_storage[1..17].copy_from_slice(&left);
        right_storage[3..19].copy_from_slice(&right);
        let mut output = [0xcc; 134];
        let masks = unsafe {
            memory::simd::unaligned(
                left_storage.as_ptr().add(1),
                right_storage.as_ptr().add(3),
                output.as_mut_ptr().add(3),
            )
        };
        let mut expected = [0_u8; 128];
        for index in 0..16 {
            expected[index] = left[index] & right[index];
            expected[16 + index] = left[index] | right[index];
            expected[32 + index] = left[index] ^ right[index];
            expected[48 + index] = !left[index] & right[index];
            expected[64 + index] = if left[index] == right[index] { 0xff } else { 0 };
            expected[112 + index] = if (left[index] as i8) > (right[index] as i8) {
                0xff
            } else {
                0
            };
        }
        for width in [2, 4] {
            let offset = if width == 2 { 80 } else { 96 };
            for index in (0..16).step_by(width) {
                let equal = left[index..index + width] == right[index..index + width];
                expected[offset + index..offset + index + width].fill(if equal { 0xff } else { 0 });
            }
        }
        assert_eq!(&output[..3], &[0xcc; 3]);
        assert_eq!(&output[3..131], &expected);
        assert_eq!(&output[131..], &[0xcc; 3]);
        let left_mask = (0..16).fold(0, |mask, index| {
            mask | (((left[index] >> 7) as i32) << index)
        });
        let equal_mask = (0..16).fold(0, |mask, index| {
            mask | (((left[index] == right[index]) as i32) << index)
        });
        assert_eq!(masks, (left_mask, equal_mask));
        let indices = [0, 15, 16, 31, 1, 17, 14, 30, 7, 23, 8, 24, 0, 31, 16, 15];
        let shuffled: [u8; 16] = std::array::from_fn(|index| {
            let source = indices[index];
            if source < 16 {
                left[source]
            } else {
                right[source - 16]
            }
        });
        assert_eq!(memory::simd::shuffle(left, right), shuffled);
        for needle in [0, 1, 0x7f, 0x80, 0xff] {
            let mask = (0..16).fold(0_u64, |mask, index| {
                mask | (((left[index] == needle) as u64) << index)
            });
            assert_eq!(
                memory::simd::portable_masks(left, needle),
                (mask, mask & 0xff, mask | 0xffff_0000)
            );
        }
        for seed in [0, 1, 0x1234_5678_9abc_def0_u64, u64::MAX] {
            let middle = seed.wrapping_add(7);
            let suffix = seed.wrapping_add(11);
            let expected = mixed_expected(seed, left, middle, right, suffix);
            let direct =
                unsafe { memory::simd::mixed(seed, vector(left), middle, vector(right), suffix) };
            let internal = unsafe { memory::simd::call_mixed(vector(left), vector(right), seed) };
            let indirect =
                unsafe { memory::simd::call_callback(callback, vector(left), vector(right), seed) };
            assert_eq!(bytes(direct), expected);
            assert_eq!(bytes(internal), expected);
            assert_eq!(bytes(indirect), expected);
            let scalar = scalar_bytes(
                seed.wrapping_add(2 * 3 + 3 * 5 + 4 * 7),
                5 + 6 * 11 + 7 * 13 + 8 * 17,
            );
            let stacked = unsafe {
                memory::simd::mixed_stack(seed, vector(left), 2, 3, 4, 5, vector(right), 6, 7, 8)
            };
            let expected: [u8; 16] =
                std::array::from_fn(|index| (left[index] | right[index]) ^ scalar[index]);
            assert_eq!(bytes(stacked), expected);
        }
        println!("simd-case {case} masks {masks:?} shuffled {shuffled:?}");
    }
    for index in 0..16 {
        let mut value = [0x7f; 16];
        value[index] = 0x80;
        assert_eq!(unsafe { memory::simd::movemask(vector(value)) }, 1 << index);
    }
    println!("simd-abi mixed GPR/XMM, callback and stack scalar arguments verified");
    println!("simd-movemask all 16 lane positions verified");
}
