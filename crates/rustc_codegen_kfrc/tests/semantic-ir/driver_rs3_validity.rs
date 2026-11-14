use core::mem::MaybeUninit;
use semantic_values::rs3_validity_probe::*;

fn even(value: u32) -> bool {
    value % 2 == 0
}

fn above_ten(value: u32) -> bool {
    value > 10
}

pub fn check() {
    for selector in 0..8u8 {
        let middle = make_middle(selector);
        let by_value = classify_middle(middle);
        let by_reference = classify_middle_ref(&middle);
        let optional = classify_optional_middle(Some(middle));
        let expected = match selector % 4 {
            0 => (10, 60),
            1 => (21, 41),
            2 => (20, 40),
            _ => (30, 50),
        };
        assert_eq!((by_value, by_reference), expected);
        assert_eq!(optional, by_value);
        println!("rs3-validity-middle {selector} {by_value} {by_reference} {optional}");
    }
    assert_eq!(classify_optional_middle(None), 0);
    for selector in 0..10u8 {
        let wrapped = make_wrapped(selector);
        let by_value = classify_wrapped(wrapped);
        let by_reference = classify_wrapped_ref(&wrapped);
        let expected = match selector % 5 {
            0 => (1, 2),
            1 => (2, 254),
            2 => (3, 9),
            3 => (4, 7),
            _ => (5, 9),
        };
        assert_eq!((by_value, by_reference), expected);
        println!("rs3-validity-wrapped {selector} {by_value} {by_reference}");
    }
    for (value, expected) in [
        (None, 0),
        (Some(None), 1),
        (Some(Some(false)), 2),
        (Some(Some(true)), 3),
    ] {
        assert_eq!(nested_option(value), expected);
    }
    for left in [0u32, 5, u32::MAX] {
        for right in [None, Some(0u32), Some(5), Some(u32::MAX)] {
            let got = optional_ordering(left, right);
            let want = right.map_or(9, |right| left.cmp(&right) as i32);
            assert_eq!(got, want);
            println!("rs3-validity-ordering {left} {right:?} {got}");
        }
    }
    let callbacks: [fn(u32) -> bool; 2] = [even, above_ten];
    for (index, callback) in callbacks.into_iter().enumerate() {
        for value in [0u32, 1, 10, 11, 12, u32::MAX] {
            let flag = callback(value);
            let negated = branch_on_negated_call(callback, value);
            let double = branch_on_double_negation(callback, value);
            let pair = negated_flag_value(callback, value);
            assert_eq!(negated, if !flag { value.wrapping_add(3) } else { value ^ 5 });
            assert_eq!(double, if flag { 11 } else { 13 });
            assert_eq!(pair, (!flag, u8::from(!flag)));
            println!("rs3-validity-branch {index} {value} {negated} {double} {pair:?}");
        }
    }
    for flag in [false, true] {
        let casts = bool_casts(flag);
        assert_eq!(casts, (flag as i8, flag as u32, flag as i64));
        println!("rs3-validity-bool {flag} {casts:?}");
    }
    for byte in [0u8, 1, 0x7f, 0x80, 0xff] {
        for signed in [i8::MIN, -1, 0, 1, i8::MAX] {
            for character in ['\0', 'a', '\u{d7ff}', '\u{e000}', '\u{10ffff}'] {
                let half = u16::from(byte) << 8 | u16::from(signed as u8);
                let got = widening_casts(byte, signed, half, character);
                assert_eq!(
                    got,
                    (
                        byte as u32,
                        byte as i32,
                        signed as i64,
                        half as u64,
                        character as u32
                    )
                );
            }
            println!("rs3-validity-widen {byte} {signed}");
        }
    }
    for word in [0u32, 0x3f, 0x40, 0xff, 0x1234_5678, u32::MAX] {
        for signed in [i32::MIN, -129, -1, 0, 127, 128, i32::MAX] {
            let got = narrowing_casts(word, signed);
            assert_eq!(
                got,
                (
                    (word & 0x3f) as u8,
                    word as u8,
                    signed as i8,
                    (word >> 20) as u16,
                    ((signed >> 28) & 0x7) as i8
                )
            );
            println!("rs3-validity-narrow {word} {signed} {got:?}");
        }
    }
    for a in [0u8, 1, 0x7f, 0x80, 0xff] {
        for c in [i8::MIN, -1, 0, 1, i8::MAX] {
            let (b, d) = (a.rotate_left(3) ^ 0x5a, c.wrapping_mul(3));
            let got = bitwise_narrow(a, b, c, d);
            assert_eq!(got, (a & b, a | b, a ^ b, c & d, c | d, c ^ d));
            for amount in [0u32, 1, 7, 8, 15, 16, 31] {
                let half = u16::from(a) << 8 | u16::from(b);
                let shifted = shifts_narrow(half, c as i16 * 257, amount);
                assert_eq!(
                    shifted,
                    (
                        half.wrapping_shr(amount),
                        (c as i16 * 257).wrapping_shr(amount),
                        (half as u8).wrapping_shr(amount)
                    )
                );
            }
            let divided = division_narrow(a, b, (c as i16).wrapping_mul(300), d as i16);
            println!("rs3-validity-bitwise {a} {c} {got:?} {divided:?}");
        }
    }
    for byte in [0u8, 1, 2, 3, 0xfe, 0xff] {
        let storage = MaybeUninit::new(byte);
        let got = raw_byte_branch(&storage);
        assert_eq!(got, if byte ^ 1 != 0 { u32::from(byte) + 100 } else { 7 });
        println!("rs3-validity-raw {byte} {got}");
    }
}
