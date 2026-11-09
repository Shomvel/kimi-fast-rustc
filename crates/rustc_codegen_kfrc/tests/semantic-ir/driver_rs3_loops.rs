use semantic_values::rs3_loops_probe::*;

pub fn check() {
    let source = [0u32, 1, 17, 99, u32::MAX, 37, 0, 41];
    for length in 0..=source.len() {
        let values = &source[..length];
        let sum: u64 = values.iter().map(|&x| u64::from(x)).sum();
        assert_eq!(while_sum(values), sum);
        assert_eq!(for_sum(values), sum);
        let enumerated = values.iter().enumerate().fold(0u64, |sum, (i, &x)| {
            sum.wrapping_add(i as u64 ^ u64::from(x))
        });
        assert_eq!(enumerate_sum(values), enumerated);
        for size in [1usize, 2, 3, 11] {
            let expected: u64 = values.chunks(size).map(|part| u64::from(part[0])).sum();
            assert_eq!(chunks_sum(values, size), expected);
        }
        for rounds in [0usize, 1, 3] {
            let mut expected = 0u64;
            for round in 0..rounds {
                for &value in values {
                    if value == 0 {
                        continue;
                    }
                    if value == u32::MAX {
                        break;
                    }
                    expected = expected.wrapping_add(u64::from(value) ^ round as u64);
                }
            }
            assert_eq!(nested_break_continue(values, rounds), expected);
        }
        let mut actual = values.to_vec();
        assert_eq!(length_after_push(&mut actual, 503), 503);
        let mut expected = values.to_vec();
        expected.push(503);
        assert_eq!(actual, expected);
        println!("rs3-loops {length} {sum}");
    }
}
