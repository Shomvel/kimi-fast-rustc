use semantic_values::rs3_conditions_probe::*;

pub fn check() {
    let bytes = [
        0u8, 1, 17, 255, 41, 53, 67, 71, 83, 97, 101, 113, 127, 131, 149, 157,
    ];
    for length in [0, 1, 4, 16] {
        let slice = &bytes[..length];
        for index in 0..=length + 1 {
            let expected = slice.get(index).copied().unwrap_or(0);
            assert_eq!(bounds_after_guard(slice, index), expected);
            if index < length {
                assert_eq!(repeated_bounds(slice, index), expected.wrapping_mul(2));
            }
        }
        println!("rs3-condition-bounds {length}");
    }
    for index in 0..16 {
        assert_eq!(unsafe { assumed_bounds(&bytes, index) }, bytes[index]);
    }
    for predicate in [false, true] {
        for value in [0u64, 17, u64::MAX] {
            assert_eq!(
                negated_guard(predicate, value),
                if predicate {
                    value.wrapping_add(1)
                } else {
                    value
                }
            );
        }
        for choose in [false, true] {
            let first = if choose {
                if predicate { 10 } else { 20 }
            } else {
                30
            };
            assert_eq!(
                non_dominating(predicate, choose),
                first + if predicate { 1 } else { 2 }
            );
        }
        println!("rs3-condition-negation {predicate}");
    }
    for initial in [0usize, 1, 4, 9, usize::MAX] {
        for count in [0usize, 1, 4, 10] {
            let mut limit = initial;
            let mut expected = 0;
            for index in 0..count {
                if index < limit {
                    expected += 1;
                }
                limit = limit.saturating_sub(1);
                if index < limit {
                    expected += 2;
                }
            }
            assert_eq!(changing_loop_limit(initial, count), expected);
        }
        println!("rs3-condition-loop {initial}");
    }
    for length in [0usize, 1, 4, 16] {
        for index in 0..=length + 1 {
            let mut actual = bytes[..length].to_vec();
            let mut expected = actual.clone();
            let before = index < length;
            expected.push(47);
            let value = expected
                .get(index)
                .map(|v| v.wrapping_add(u8::from(before)))
                .unwrap_or(0);
            assert_eq!(reloaded_length(&mut actual, index), value);
            assert_eq!(actual, expected);
        }
        println!("rs3-condition-reload {length}");
    }
    let array = [0u32, 17, u32::MAX, 99, 23, 37, 41, 53];
    for index in [0usize, 1, 3, 4, 7, 8, usize::MAX] {
        assert_eq!(
            array_guard(&array, index),
            if index < 4 { array[index] } else { 0 }
        );
    }
    for a in [0u64, 1, 17, u64::MAX] {
        for b in [0u64, 1, 17, u64::MAX] {
            assert_eq!(comparison_family(a, b), if a < b { 7 } else { 1 });
        }
        println!("rs3-condition-family {a}");
    }
    for value in [i64::MIN, -17, -5, -4, -1, 0, 7, 8, i64::MAX] {
        assert_eq!(signed_threshold(value), u64::from(value < 0));
    }
    println!("rs3-condition-thresholds");
}
