fn lookup(value: u64, cases: &[(u64, u64)], otherwise: u64) -> u64 {
    for &(case, answer) in cases {
        if value == case {
            return answer;
        }
    }
    otherwise
}

fn main() {
    let dense = [(10, 37), (11, 11), (12, 92), (13, 4), (14, 71), (15, 55)];
    let signed = [
        (128, 41),
        (253, 27),
        (254, 91),
        (255, 5),
        (0, 66),
        (1, 80),
        (127, 19),
    ];
    let sparse = [
        (0, 13),
        (99, 72),
        (4096, 45),
        (0xffff_ffff, 34),
        (0x1_0000_0000, 91),
        (0x8000_0000_0000_0000, 83),
        (u64::MAX, 51),
    ];
    let mut checksum = 0u64;
    for value in 0..=255u64 {
        let got = switches::dense8(value as u8);
        assert_eq!(got, lookup(value, &dense, 777));
        let got_signed = switches::signed8(value as u8 as i8);
        assert_eq!(got_signed, lookup(value, &signed, 999));
        checksum = checksum.wrapping_add(got ^ got_signed);
    }
    let mut samples = vec![0, 1, 98, 99, 100, 4095, 4096, 4097, u64::MAX];
    for &(case, _) in &sparse {
        samples.extend([case.wrapping_sub(1), case, case.wrapping_add(1)]);
    }
    let mut state = 0x1739_42c1_51a8_e6ddu64;
    for _ in 0..256 {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        samples.push(state);
    }
    for value in samples {
        let got = switches::sparse64(value);
        assert_eq!(got, lookup(value, &sparse, 888));
        checksum = checksum.wrapping_add(got);
    }
    for salt in [0, 17, 0xffff_ffff, u64::MAX] {
        for value in 0..16u64 {
            let cases = [(3, 17), (4, 9), (5, salt), (6, 81), (7, 33)];
            let got = switches::shared_phi(value as u32, salt);
            assert_eq!(got, lookup(value, &cases, 101) ^ salt);
            checksum = checksum.wrapping_add(got);
        }
    }
    for value in 0..16u64 {
        let expected = if value == 1 {
            91
        } else if value == 2 {
            73
        } else {
            let added = value.wrapping_add(11);
            added ^ lookup(value, &[(3, 27), (4, 41)], 88)
        };
        assert_eq!(switches::fixed_event(value), expected);
    }
    for start in [0u64, 1, 7, 19, u64::MAX] {
        for count in [0, 1, 2, 17, 100] {
            let mut value = start;
            let mut expected = 0u64;
            for index in 0..count {
                expected = expected.wrapping_add(lookup(
                    value & 7,
                    &[(0, 31), (1, 19), (2, 73), (3, 11), (4, 97), (5, 42)],
                    88,
                ));
                value = value.rotate_left(5).wrapping_add(index as u64);
            }
            let got = switches::loop_dispatch(start, count);
            assert_eq!(got, expected);
            checksum = checksum.wrapping_add(got);
        }
    }
    println!("switches checksum {checksum:016x}");
}
