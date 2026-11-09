//! base64 0.22.1 driver: tight byte loops (GeneralPurpose internal_encode / internal_decode live in the library).
use base64::engine::general_purpose::{STANDARD, STANDARD_NO_PAD, URL_SAFE, URL_SAFE_NO_PAD};
use base64::Engine as _;
use std::{hint::black_box, time::Instant};

fn bytes(len: usize, seed: u64) -> Vec<u8> {
    let mut x = seed.wrapping_mul(0x9E3779B97F4A7C15) | 1;
    (0..len).map(|_| { x ^= x << 13; x ^= x >> 7; x ^= x << 17; (x >> 24) as u8 }).collect()
}

fn run(mode: &str, iterations: u64) -> u64 {
    let mut checksum = 0u64;
    match mode {
        "encode" => {
            let data = bytes(4096, 1);
            let mut out = vec![0u8; 8192];
            for i in 0..iterations {
                let n = STANDARD.encode_slice(black_box(&data[..]), black_box(&mut out[..])).unwrap();
                checksum = checksum.wrapping_add(n as u64 + out[(i as usize * 31) % n] as u64);
            }
        }
        "decode" => {
            let text = STANDARD.encode(bytes(4096, 2));
            let mut out = vec![0u8; 4096 + 64];
            for i in 0..iterations {
                let n = STANDARD.decode_slice(black_box(text.as_bytes()), black_box(&mut out[..])).unwrap();
                checksum = checksum.wrapping_add(n as u64 + out[(i as usize * 31) % n] as u64);
            }
        }
        "short" => {
            let inputs: Vec<Vec<u8>> = (0..64).map(|n| bytes(n, n as u64 + 3)).collect();
            for i in 0..iterations {
                let d = &inputs[i as usize % inputs.len()];
                let s = URL_SAFE_NO_PAD.encode(black_box(d));
                let v = URL_SAFE_NO_PAD.decode(black_box(s.as_bytes())).unwrap();
                checksum = checksum.wrapping_add(s.len() as u64 * 3 + v.len() as u64 + *v.first().unwrap_or(&7) as u64);
            }
        }
        _ => panic!("unknown workload {mode}"),
    }
    checksum
}

fn fnv(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf29ce484222325u64, |h, b| (h ^ *b as u64).wrapping_mul(0x100000001b3))
}

fn corpus() {
    let engines = [("std", &STANDARD), ("std-nopad", &STANDARD_NO_PAD), ("url", &URL_SAFE), ("url-nopad", &URL_SAFE_NO_PAD)];
    for (name, engine) in engines {
        for len in (0..=80).chain([255, 256, 257, 1000, 4095, 4096, 4097]) {
            let data = bytes(len, len as u64 * 7 + 1);
            let text = engine.encode(&data);
            let back = engine.decode(&text).map(|v| v == data);
            let mut slice = vec![0u8; text.len() + 4];
            let n = engine.encode_slice(&data, &mut slice).unwrap();
            if len <= 20 || len % 17 == 0 {
                println!("{name} {len} {text} {back:?} {n}");
            } else {
                println!("{name} {len} {:016x} {} {back:?} {n}", fnv(text.as_bytes()), text.len());
            }
        }
        for bad in ["a", "ab=c", "====", "Zm9v!", "Zg==Zg==", "Zh==", "Zm9=", "Zm9vYg", "Zm9vYg=", "Zm9vYg==", "Zm9vYmE=", "-_-_", "+/+/", "YQ", "YQ=", "Y Q==", "\u{e9}Q=="] {
            match engine.decode(bad) {
                Ok(v) => println!("{name} decode {bad:?} ok {v:?}"),
                Err(e) => println!("{name} decode {bad:?} err {e} {e:?}"),
            }
            let mut small = [0u8; 2];
            println!("{name} decode_slice {bad:?} {:?}", engine.decode_slice(bad, &mut small).map_err(|e| e.to_string()));
        }
    }
    let mut buf = String::new();
    for i in 0..50 {
        STANDARD.encode_string(bytes(i, 99), &mut buf);
    }
    println!("encode_string {} {:016x}", buf.len(), fnv(buf.as_bytes()));
    println!("decoded_len_estimate {:?}", (0..40).map(base64::decoded_len_estimate).collect::<Vec<_>>());
    println!("encoded_len {:?}", (0..20).map(|n| base64::encoded_len(n, n % 2 == 0)).collect::<Vec<_>>());
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("corpus") => corpus(),
        Some("run") => {
            let mode = args[2].as_str();
            let iterations: u64 = args[3].parse().unwrap();
            let start = Instant::now();
            let checksum = run(mode, iterations);
            let elapsed = start.elapsed().as_nanos();
            println!("{{\"workload\":\"{mode}\",\"iterations\":{iterations},\"checksum\":{checksum},\"elapsed_ns\":{elapsed}}}");
        }
        _ => panic!("usage: driver corpus | driver run <workload> <iterations>"),
    }
}
