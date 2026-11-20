use semantic_values::range_fusion_probe::*;

pub fn check() {
    let mut characters = 0;
    for bits in 0..=0x10ffff {
        let Some(value) = char::from_u32(bits) else {
            continue;
        };
        assert_eq!(character_digits(value), value.is_ascii_digit());
        assert_eq!(character_letters(value), value.is_ascii_alphabetic());
        assert_eq!(
            character_scalar_edges(value),
            matches!(bits, 0xd7ff | 0xe000 | 0x10fffe | 0x10ffff)
        );
        characters += 1;
    }
    assert_eq!(characters, 1_112_064);
    let mut char_trace = 0u64;
    let mut expected_char_trace = 0u64;
    for value in [
        '\0',
        '\t',
        '@',
        'A',
        'Z',
        '[',
        'a',
        'z',
        '\u{d7ff}',
        '\u{e000}',
        '\u{10ffff}',
    ] {
        if value >= 'A' {
            expected_char_trace = expected_char_trace.wrapping_add(value as u64);
        }
        assert_eq!(
            character_effect_between_bounds(value, &mut char_trace),
            value.is_ascii_uppercase()
        );
        assert_eq!(char_trace, expected_char_trace);
    }
    println!("char-range scalar-values={characters} trace={char_trace}");
    let bytes: Vec<u8> = (0..=u8::MAX).collect();
    for &value in &bytes {
        let ascii = (b'A'..=b'Z').contains(&value);
        assert_eq!(ascii_range(value), ascii);
        assert_eq!(ascii_outside(value), !ascii);
        assert_eq!(
            ascii_letters(value),
            ascii || (b'a'..=b'z').contains(&value)
        );
        assert_eq!(
            signed_range(value as i8),
            (-64..=64).contains(&(value as i8))
        );
    }
    let expected: usize = bytes
        .iter()
        .map(|&v| {
            usize::from((b'A'..=b'Z').contains(&v))
                + 3 * usize::from((-7..=11).contains(&(v as i8)))
        })
        .sum();
    assert_eq!(loop_ranges(&bytes), expected);
    let mut samples = vec![
        0,
        1,
        16,
        17,
        23,
        24,
        91,
        92,
        1u64 << 63,
        i64::MAX as u64,
        (-38i64) as u64,
        (-37i64) as u64,
        u64::MAX - 501,
        u64::MAX - 500,
        u64::MAX - 5,
        u64::MAX - 4,
        u64::MAX,
    ];
    let mut random = 0x1234_5678_9abc_def0u64;
    for _ in 0..1024 {
        random = random.wrapping_mul(6364136223846793005).wrapping_add(1);
        samples.push(random);
    }
    let mut trace = 0u64;
    let mut expected_trace = 0u64;
    for value in samples.iter().copied() {
        assert_eq!(
            signed_word_range(value as i64),
            (-37..=91).contains(&(value as i64))
        );
        assert_eq!(
            high_word_range(value),
            (u64::MAX - 500..=u64::MAX - 5).contains(&value)
        );
        assert_eq!(
            different_failure_values(value),
            if value < 17 {
                1
            } else if value > 23 {
                2
            } else {
                0
            }
        );
        if value >= 17 {
            expected_trace = expected_trace.wrapping_add(value);
        }
        assert_eq!(
            effect_between_bounds(value, &mut trace),
            (17..=23).contains(&value)
        );
        assert_eq!(trace, expected_trace);
    }
    println!(
        "range-fusion bytes={} words={} loop={expected} trace={trace}",
        bytes.len(),
        samples.len()
    );
}
