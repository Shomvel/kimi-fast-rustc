extern crate byte_ladders as subject;

fn oracle(input: &str) -> u8 {
    let alternatives: &[&[&str]] = &[
        &["nanos", "nsec", "ns"],
        &["usec", "us", "µs"],
        &["millis", "msec", "ms"],
        &["seconds", "second", "secs", "sec", "s"],
        &["minutes", "minute", "min", "mins", "m"],
        &["hours", "hour", "hr", "hrs", "h"],
        &["days", "day", "d"],
        &["weeks", "week", "wk", "wks", "w"],
        &["months", "month", "M"],
        &["years", "year", "yr", "yrs", "y"],
    ];
    alternatives
        .iter()
        .position(|group| group.contains(&input))
        .map_or(0, |index| index as u8 + 1)
}

fn check(input: &[u8], checksum: &mut u64) {
    let patterns: &[&[u8]] = &[b"alpha", b"xy", b"bravo", b"z", b"rs", b"delta", b"q"];
    let expected = patterns
        .iter()
        .position(|pattern| *pattern == input)
        .map_or(0, |index| index as u8 + 1);
    assert_eq!(subject::byte_code(input), expected, "bytes={input:?}");
    *checksum = checksum.wrapping_mul(37).wrapping_add(u64::from(expected));
    if let Ok(text) = std::str::from_utf8(input) {
        let expected = oracle(text);
        assert_eq!(subject::unit_code(text), expected, "text={text:?}");
        *checksum = checksum.wrapping_mul(37).wrapping_add(u64::from(expected));
    }
}

fn main() {
    let samples = [
        "nanos",
        "nsec",
        "ns",
        "usec",
        "us",
        "µs",
        "millis",
        "msec",
        "ms",
        "seconds",
        "second",
        "secs",
        "sec",
        "s",
        "minutes",
        "minute",
        "min",
        "mins",
        "m",
        "hours",
        "hour",
        "hr",
        "hrs",
        "h",
        "days",
        "day",
        "d",
        "weeks",
        "week",
        "wk",
        "wks",
        "w",
        "months",
        "month",
        "M",
        "years",
        "year",
        "yr",
        "yrs",
        "y",
        "alpha",
        "xy",
        "bravo",
        "z",
        "rs",
        "delta",
        "q",
        "",
        "é",
        "😀",
        "microseconds",
    ];
    let mut checksum = 0u64;
    for input in samples {
        check(input.as_bytes(), &mut checksum);
        for offset in 0..input.len() {
            for byte in 0..=u8::MAX {
                let mut changed = input.as_bytes().to_vec();
                changed[offset] = byte;
                check(&changed, &mut checksum);
            }
        }
    }
    for left in 0..=u8::MAX {
        check(&[left], &mut checksum);
        for right in 0..=u8::MAX {
            check(&[left, right], &mut checksum);
        }
    }
    let mut state = 0x25_27_39_51u64;
    for _ in 0..65_536 {
        state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
        let mut bytes = vec![0; (state as usize >> 32) & 31];
        for byte in &mut bytes {
            state = state.rotate_left(7).wrapping_add(17);
            *byte = (state >> 23) as u8;
        }
        check(&bytes, &mut checksum);
    }
    println!("byte-ladder differential {checksum}");
}
