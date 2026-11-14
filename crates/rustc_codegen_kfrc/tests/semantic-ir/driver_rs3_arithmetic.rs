use semantic_values::rs3_arithmetic_probe::*;

pub fn check() {
    for a in [0u32, 1, 17, 65535, u32::MAX] {
        for b in [0u32, 1, 37, u32::MAX] {
            if let Some(sum) = a.checked_add(b) {
                assert_eq!(unsafe { unchecked_add_u32(a, b) }, u64::from(sum));
            }
            assert_eq!(wrapping_add_u32(a, b), u64::from(a.wrapping_add(b)));
            assert_eq!(checked_add_u32(a, b), a.checked_add(b));
        }
        for amount in [0, 1, 7, 16, 31] {
            for signed in [i32::MIN, -17, 0, 17, i32::MAX] {
                assert_eq!(
                    unsafe { unchecked_shifts(a, signed, amount) },
                    (a.wrapping_shl(amount), a >> amount, signed >> amount)
                );
            }
        }
        println!("rs3-arithmetic-u32 {a}");
    }
    for a in [i32::MIN, -65536, -17, 0, 1, 17, 65535, i32::MAX] {
        for b in [i32::MIN, -17, -1, 0, 1, 17, i32::MAX] {
            if let Some(difference) = a.checked_sub(b) {
                assert_eq!(unsafe { unchecked_sub_i32(a, b) }, i64::from(difference));
            }
            if let Some(product) = a.checked_mul(b) {
                assert_eq!(unsafe { unchecked_mul_i32(a, b) }, i64::from(product));
            }
        }
        println!("rs3-arithmetic-i32 {a}");
    }
    let values = [[0u8; 24]; 17];
    let base = values.as_ptr();
    for first in 0..=17 {
        for second in 0..=17 {
            let a = unsafe { base.add(first) };
            let b = unsafe { base.add(second) };
            assert_eq!(
                unsafe { distance24_signed(a, b) },
                first as isize - second as isize
            );
            if first >= second {
                assert_eq!(unsafe { distance24_unsigned(a, b) }, first - second);
            }
        }
        assert_eq!(iterator_length24(&values[..first]), first);
    }
    println!("rs3-arithmetic-exact-distance");
}
