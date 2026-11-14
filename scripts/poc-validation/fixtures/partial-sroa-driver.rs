use partial_sroa::*;
use std::{hint::black_box, time::Instant};

fn sample(family: &str, seed: u64, rounds: usize) -> u64 {
    let mut output = !seed;
    let value = match family {
        "retyped" => retyped_sibling(seed, rounds),
        "copy" => copied_sibling(seed, rounds, &mut output),
        "nested" => nested_copy(seed, rounds, &mut output),
        "array2" => array_two(seed, rounds, &mut output),
        "array5" => array_five(seed, rounds, &mut output),
        "array11" => array_eleven(seed, rounds, &mut output),
        _ => panic!("unknown family"),
    };
    value ^ output.rotate_left(13)
}

fn main() {
    let args: Vec<_> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("run") {
        let family = &args[2];
        let iterations: usize = args[3].parse().unwrap();
        let mut checksum = 0u64;
        let start = Instant::now();
        for i in 0..iterations {
            checksum = checksum.wrapping_add(sample(family, black_box(i as u64), black_box(64)));
        }
        println!(
            "{{\"elapsed_ns\":{},\"checksum\":{checksum}}}",
            start.elapsed().as_nanos()
        );
        return;
    }
    let mut state = 0xbac6_e393_4a97_1d05u64;
    let mut checksum = 0u64;
    let mut cases = 0usize;
    for i in 0..256 {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        let seed = match i {
            0 => 0,
            1 => u64::MAX,
            2 => 1 << 63,
            _ => state,
        };
        for family in ["retyped", "copy", "nested", "array2", "array5", "array11"] {
            for rounds in [0, 1, 2, 7, 31] {
                checksum = checksum.rotate_left(7) ^ sample(family, seed, rounds);
                cases += 1;
            }
        }
    }
    println!("cases={cases} checksum={checksum}");
}
