use conditional_returns::*;
use std::sync::atomic::{AtomicU64, Ordering};

static OBSERVATIONS: AtomicU64 = AtomicU64::new(0);

#[unsafe(no_mangle)]
pub extern "Rust" fn conditional_observe(value: u64) -> u64 {
    OBSERVATIONS.fetch_add(1, Ordering::Relaxed);
    value | 1
}

fn expected(input: &[u8]) -> u64 {
    match input {
        b"ns" => 1,
        b"microseconds" => 2,
        b"ms" => 3,
        b"second" => 4,
        b"minute" => 5,
        b"hour" => 6,
        b"day" => 7,
        b"week" => 8,
        _ => 0,
    }
}

fn main() {
    let samples: &[&[u8]] = &[
        b"",
        b"n",
        b"ns",
        b"ms",
        b"microseconds",
        b"second",
        b"minute",
        b"hour",
        b"day",
        b"week",
        b"nanoseconds",
        b"seconds",
        b"MICROSECONDS",
        b"\0s",
        b"\xff\xff",
    ];
    let mut checksum = 0u64;
    for sample in samples {
        assert_eq!(classify(sample), expected(sample));
        checksum = checksum.wrapping_add(classify(sample));
    }
    let mut state = 0xe781_0279_53ac_8bd1u64;
    for case in 0..10_006usize {
        let length = case % 33;
        let mut bytes = [0u8; 32];
        for byte in &mut bytes {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            *byte = state as u8;
        }
        let input = &bytes[..length];
        assert_eq!(classify(input), expected(input));
        let before = OBSERVATIONS.load(Ordering::Relaxed);
        assert_eq!(guarded_observation(length), length == 7);
        assert_eq!(
            OBSERVATIONS.load(Ordering::Relaxed),
            before + u64::from(length == 7)
        );
        let before = OBSERVATIONS.load(Ordering::Relaxed);
        assert_eq!(required_observation(length), length == 7);
        assert_eq!(OBSERVATIONS.load(Ordering::Relaxed), before + 1);
        if length != 17 {
            // Raw pointers are values even when dangling. The unequal branch
            // must not introduce the equal arm's memory access.
            assert!(!unsafe { unequal_lengths_do_not_read(1usize as *const u8, length) });
        }
        checksum ^= classify(input).wrapping_add(state);
    }
    for text in ["", "a", "hello", "微秒", "🌍🦀"] {
        assert_eq!(
            metadata_roundtrip(text),
            text.len() ^ text.len().rotate_left(7)
        );
        for next in ["", "replacement", "秒"] {
            let mut slot = text;
            assert_eq!(
                unsafe { metadata_from_snapshot(&mut slot, next) },
                (text.len(), next.len())
            );
            assert_eq!(slot, next);
        }
    }
    println!(
        "conditional returns: 10006 generated cases, observations and metadata snapshots passed; checksum={checksum:016x} observations={}",
        OBSERVATIONS.load(Ordering::Relaxed)
    );
}
