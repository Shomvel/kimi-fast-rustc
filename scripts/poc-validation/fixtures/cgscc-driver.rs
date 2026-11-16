use cgscc::{Pair, SALT, TAG};
use std::sync::atomic::{AtomicU64, Ordering};

static OBSERVATIONS: AtomicU64 = AtomicU64::new(0);
static TRACE: AtomicU64 = AtomicU64::new(0x3f2e_9517_bc04_d6a8);

#[unsafe(no_mangle)]
pub extern "Rust" fn cgscc_observe(value: u64) -> u64 {
    OBSERVATIONS.fetch_add(1, Ordering::Relaxed);
    let trace = TRACE.load(Ordering::Relaxed);
    TRACE.store(trace.rotate_left(11) ^ value, Ordering::Relaxed);
    value.rotate_left(5).wrapping_add(0xa17c_5e09_382b_640d)
}

#[unsafe(no_mangle)]
pub extern "C" fn cgscc_external_pair(value: Pair) -> Pair {
    Pair {
        value: value.value.rotate_right(13),
        tag: value.tag ^ 0x73b2_08df_a941_6ec5,
    }
}

fn expected_recursive(mut depth: u64, mut value: u64) -> u64 {
    let mut in_a = true;
    while depth != 0 {
        if in_a {
            value ^= depth;
        } else {
            value = value.rotate_left(3);
        }
        in_a = !in_a;
        depth -= 1;
    }
    if in_a {
        value
    } else {
        !value
    }
}

fn main() {
    let mut state = 0x952b_6d07_fa43_8ec1u64;
    let mut checksum = 0u64;
    let mut expected_trace = TRACE.load(Ordering::Relaxed);
    let mut expected_observations = 0;
    for case in 0..10_006 {
        let value = match case {
            0 => 0,
            1 => 1,
            2 => u64::MAX,
            3 => 1 << 63,
            4 => 0x0102_0304_0506_0708,
            5 => 0x807f_ff00_55aa_fe01,
            _ => {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                state
            }
        };
        let expected = Pair {
            value: value ^ SALT,
            tag: TAG,
        };
        assert_eq!(cgscc::cgscc_chain(value), expected);
        let diamond = (value ^ SALT) ^ ((!value) ^ SALT).rotate_left(7);
        assert_eq!(cgscc::cgscc_diamond(value), diamond);
        assert_eq!(OBSERVATIONS.load(Ordering::Relaxed), expected_observations);
        assert_eq!(TRACE.load(Ordering::Relaxed), expected_trace);

        let tag = if case % 2 == 0 { TAG } else { value ^ TAG };
        let mut guarded = value;
        if tag != TAG {
            for _ in 0..32 {
                expected_trace = expected_trace.rotate_left(11) ^ guarded;
                guarded = guarded.rotate_left(5).wrapping_add(0xa17c_5e09_382b_640d);
            }
            expected_observations += 32;
        }
        assert_eq!(
            cgscc::cgscc_dynamic_guard(Pair { value, tag }),
            Pair {
                value: guarded,
                tag: if tag == TAG { TAG } else { 0 },
            }
        );
        assert_eq!(OBSERVATIONS.load(Ordering::Relaxed), expected_observations);
        assert_eq!(TRACE.load(Ordering::Relaxed), expected_trace);
        assert_eq!(
            cgscc::cgscc_noinline_caller(value),
            (value.rotate_left(9) ^ SALT) ^ value
        );
        let pair = Pair { value, tag: !value };
        let expected_pair = Pair {
            value: pair.value ^ pair.tag,
            tag: pair.tag.rotate_left(3),
        };
        assert_eq!(cgscc::cgscc_pair_abi(pair), expected_pair);
        // Loading the exported pointer exercises its original callable ABI.
        assert_eq!(
            std::hint::black_box(cgscc::PAIR_READER)(pair),
            expected_pair
        );
        assert_eq!(
            cgscc::cgscc_external_caller(pair),
            cgscc_external_pair(pair)
        );
        let depth = (case % 129) as u64;
        assert_eq!(
            cgscc::cgscc_recursive(depth, value),
            expected_recursive(depth, value)
        );
        checksum = checksum.wrapping_add(expected.value ^ diamond ^ guarded ^ expected_trace);
    }
    println!(
        "cgscc: 10006 cases, seven-level/diamond calls, effects, recursive and ABI boundaries passed; checksum={checksum:016x} observations={expected_observations} trace={expected_trace:016x}"
    );
}
