use core::{num::NonZeroU32, ptr::NonNull};
use semantic_values::rs3_ranges_probe::*;

pub fn check() {
    let mut bytes = [0u8; 256];
    let words = [0u32; 256];
    for length in 0..=bytes.len() {
        assert!(byte_slice_bounded(&bytes[..length]));
        assert!(mutable_byte_slice_bounded(&mut bytes[..length]));
        assert!(word_slice_bounded(&words[..length]));
    }
    let text = "slice 长度";
    for length in 0..=text.len() {
        if text.is_char_boundary(length) {
            assert!(string_bounded(&text[..length]));
        }
    }
    // This ZST reference is valid even at usize::MAX length. A raw slice
    // pointer can also carry arbitrary metadata without being dereferenced.
    let zst =
        unsafe { core::slice::from_raw_parts(NonNull::<()>::dangling().as_ptr(), usize::MAX) };
    assert!(!zst_slice_bounded(zst));
    let raw = core::ptr::slice_from_raw_parts(core::ptr::null::<u8>(), usize::MAX);
    assert!(!raw_byte_slice_bounded(raw));
    assert!(raw_byte_slice_bounded(core::ptr::slice_from_raw_parts(
        core::ptr::null::<u8>(),
        0
    )));
    println!("rs3-range-slices lengths=257 zst=usize::MAX raw=usize::MAX");
    for bits in [1u32, 2, 17, u32::MAX] {
        assert!(nonzero(NonZeroU32::new(bits).unwrap()));
    }
    for value in ['\0', 'a', '\u{d7ff}', '\u{e000}', '界', '\u{10ffff}'] {
        assert!(character(value));
        println!("rs3-range-char {}", value as u32);
    }
    for value in [false, true] {
        assert_eq!(boolean(value), usize::from(value));
    }
    let mut byte = 17u8;
    assert!(!nonnull(NonNull::from(&mut byte)));
    assert!(!nonnull(NonNull::dangling()));
    for bits in [0u32, 1, 37, u32::MAX] {
        assert_eq!(raw_maybe_uninit(bits), bits != 0);
        println!("rs3-range-raw {bits}");
    }
    let callbacks: [fn(u32) -> u32; 2] = [callback_add, callback_rotate];
    let pointers = [core::ptr::null(), core::ptr::from_ref(&byte).cast::<()>()];
    for (callback_index, callback) in callbacks.into_iter().enumerate() {
        for raw in pointers {
            for value in [0, 1, u32::MAX] {
                let expected = callback(value).wrapping_add(if raw.is_null() { 17 } else { 29 });
                let direct = callback_and_nullable_raw(callback, raw, value);
                let tuple = callback_tuple_and_nullable_raw((callback, 5), (raw, 12), value);
                let array = callback_array_and_nullable_raw([callback], [raw], value);
                assert_eq!(direct, expected);
                assert_eq!(tuple, expected.wrapping_add(5 ^ 12));
                assert_eq!(array, expected);
                println!(
                    "rs3-range-callback {callback_index} {} {value} {direct} {tuple} {array}",
                    raw.is_null()
                );
            }
        }
    }
}

fn callback_add(value: u32) -> u32 {
    value.wrapping_add(5)
}

fn callback_rotate(value: u32) -> u32 {
    value.rotate_left(7)
}
