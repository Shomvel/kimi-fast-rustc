use storage_representation::*;

fn bytes(words: [u64; 2]) -> [u8; 16] {
    let mut result = [0; 16];
    result[..8].copy_from_slice(&words[0].to_le_bytes());
    result[8..].copy_from_slice(&words[1].to_le_bytes());
    result
}

fn main() {
    let mut state = 0x0123_4567_89ab_cdefu64;
    let mut checksum = 0u64;
    for _ in 0..4096 {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        let a = state;
        let b = !state.rotate_left(19);
        let replacement = (b >> 5) as u32;
        let byte = (b >> 37) as u8;
        let mut expected = a.to_le_bytes();
        expected[2..6].copy_from_slice(&replacement.to_le_bytes());
        expected[5] = byte;
        assert_eq!(
            mixed_width(a, replacement, byte),
            u64::from_le_bytes(expected)
        );
        let source: [u8; 7] = b.to_le_bytes()[..7].try_into().unwrap();
        let mut expected = bytes([a, b]);
        expected[3..10].copy_from_slice(&source);
        assert_eq!(bytes(partial_copy([a, b], &source)), expected);
        for reverse in [false, true] {
            let mut expected = bytes([a, b]);
            if reverse {
                expected.copy_within(3..14, 0);
            } else {
                expected.copy_within(0..11, 3);
            }
            let actual = overlapping_copy(a, b, reverse);
            assert_eq!(bytes(actual), expected);
            checksum = checksum.wrapping_add(actual[0] ^ actual[1]);
            let expected = if reverse {
                Err((a, (a >> 8) as u8))
            } else {
                Ok([a, a.wrapping_add(1), a.wrapping_mul(7)])
            };
            assert_eq!(forward_result(a, reverse), expected);
            assert_eq!(forward_parameter(expected), expected);
        }
        assert_eq!(raw_snapshot(a), a);
        let mut raw = std::mem::MaybeUninit::<[u64; 3]>::uninit();
        // Transport must preserve the initialized word without requiring the
        // remaining object bytes to be initialized or valid integers.
        unsafe {
            raw.as_mut_ptr().cast::<u64>().write(b);
            let returned = forward_raw(raw);
            assert_eq!(returned.as_ptr().cast::<u64>().read(), b);
        }
    }
    println!("storage-representation {checksum}");
}
