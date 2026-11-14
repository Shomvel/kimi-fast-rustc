//! humantime 2.4.0 driver: duration/timestamp parsing and Display formatting (integer
//! arithmetic, byte parsing, core::fmt) implemented in non-inline library functions.
use humantime::{format_duration, format_rfc3339, format_rfc3339_micros, format_rfc3339_millis, format_rfc3339_nanos,
                format_rfc3339_seconds, parse_duration, parse_rfc3339, parse_rfc3339_weak};
use std::fmt::Write as _;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use std::{hint::black_box, time::Instant};

const DURATIONS: [&str; 10] = ["2h 37min", "1year 2months 3weeks 4days", "15days 2min 2s", "123ms 456us 789ns",
    "1y1M1w1d1h1m1s", "3600s", "17nsec", "2 hours 5 minutes", "bad", "1 fortnight"];
const STAMPS: [&str; 8] = ["2018-02-14T00:28:07Z", "1970-01-01T00:00:00Z", "2000-02-29T23:59:59.999999999Z",
    "9999-12-31T23:59:59Z", "2018-02-14 00:28:07", "2018-02-14T00:28:07.123Z", "2018-02-30T00:00:00Z", "2018-02-14T00:28:07+01:00"];

fn secs(t: SystemTime) -> u64 {
    let d = t.duration_since(UNIX_EPOCH).unwrap();
    d.as_secs().wrapping_mul(1_000_000_007).wrapping_add(d.subsec_nanos() as u64)
}

fn run(mode: &str, iterations: u64) -> u64 {
    let mut checksum = 0u64;
    let mut buf = String::with_capacity(128);
    for i in 0..iterations {
        let value = match mode {
            "dur-parse" => match parse_duration(black_box(DURATIONS[i as usize % DURATIONS.len()])) {
                Ok(d) => d.as_secs().wrapping_add(d.subsec_nanos() as u64),
                Err(e) => { black_box(e); 7 }
            },
            "dur-format" => {
                let d = Duration::new(i.wrapping_mul(7919) % 100_000_000, (i.wrapping_mul(104_729) % 1_000_000_000) as u32);
                buf.clear();
                write!(buf, "{}", format_duration(black_box(d))).unwrap();
                buf.len() as u64 + buf.as_bytes()[0] as u64
            }
            "ts-parse" => match parse_rfc3339_weak(black_box(STAMPS[i as usize % STAMPS.len()])) {
                Ok(t) => secs(t),
                Err(e) => { black_box(e); 11 }
            },
            "ts-format" => {
                let t = UNIX_EPOCH + Duration::new(i.wrapping_mul(86_413) % 4_000_000_000, (i.wrapping_mul(7_777_777) % 1_000_000_000) as u32);
                buf.clear();
                write!(buf, "{}", format_rfc3339_nanos(black_box(t))).unwrap();
                buf.len() as u64 + buf.as_bytes()[3] as u64
            }
            _ => panic!("unknown workload {mode}"),
        };
        checksum = checksum.wrapping_add(black_box(value));
    }
    checksum
}

fn corpus() {
    let extra = ["", "0", "0s", "1", "1 s", "5 m", "1.5h", "-1s", "18446744073709551615s", "18446744073709551616s",
        "584942417355y", "1000000000000000000000ns", "1msec 1usec 1nsec", "2weeks", "3 days 4 hours", "1y 1y",
        "12 hrs 30 mins", "1µs", "4ys", "10seconds", "  2h", "2h  ", "1d\t2h"];
    for s in DURATIONS.iter().chain(extra.iter()) {
        match parse_duration(s) {
            Ok(d) => println!("dur {s:?} ok {:?} {}", d, format_duration(d)),
            Err(e) => println!("dur {s:?} err {e} {e:?}"),
        }
    }
    let mut x = 1u64;
    for i in 0..200u64 {
        x = x.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        let d = Duration::new((x >> 20) % (if i % 3 == 0 { 100 } else { 4_000_000_000 }), ((x >> 7) % 1_000_000_000) as u32);
        let f = format_duration(d).to_string();
        let back = parse_duration(&f).map(|b| b == d);
        println!("fmt {d:?} {f} {back:?}");
    }
    let more = ["1970-01-01T00:00:00.000000001Z", "1969-12-31T23:59:59Z", "2016-12-31T23:59:60Z", "2100-02-29T00:00:00Z",
        "2400-02-29T12:00:00Z", "2018-13-01T00:00:00Z", "2018-02-14T24:00:00Z", "2018-02-14T00:28:07.Z", "2018-02-14",
        "2018-02-14t00:28:07z", "2018-02-14T00:28:07.1234567891Z", "10000-01-01T00:00:00Z"];
    for s in STAMPS.iter().chain(more.iter()) {
        let strict = parse_rfc3339(s).map(secs).map_err(|e| e.to_string());
        let weak = parse_rfc3339_weak(s).map(|t| format_rfc3339_nanos(t).to_string()).map_err(|e| e.to_string());
        println!("ts {s:?} {strict:?} {weak:?}");
    }
    for i in 0..300u64 {
        x = x.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        let t = UNIX_EPOCH + Duration::new((x >> 16) % 253_402_300_800, ((x >> 3) % 1_000_000_000) as u32);
        let a = format_rfc3339(t).to_string();
        let b = format_rfc3339_seconds(t).to_string();
        let c = format_rfc3339_millis(t).to_string();
        let d = format_rfc3339_micros(t).to_string();
        let e = format_rfc3339_nanos(t).to_string();
        let back = parse_rfc3339(&e).map(|p| p == t);
        if i % 3 == 0 { println!("ts-fmt {a} {b} {c} {d} {e} {back:?}"); } else { println!("ts-fmt {e} {back:?}"); }
    }
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
