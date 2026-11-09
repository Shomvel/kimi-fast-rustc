use semver::{Version, VersionReq, Prerelease};
use std::{hint::black_box, time::Instant};
fn main() {
    let args: Vec<_> = std::env::args().collect();
    let mode = args.get(1).map(String::as_str).unwrap_or("parse");
    let iterations: u64 = args.get(2).unwrap().parse().unwrap();
    let versions = ["1.2.3", "0.0.0", "12.34.56-alpha.7+build.123", "18446744073709551615.0.1", "1.02.3", "2.3.4-rc.1"];
    let requirements = ["^1.2.3", ">=1.0, <3", "~2.3", "*", "1.2.x", ">=0.0.1-alpha"];
    let version = Version::parse("1.2.3-alpha.7").unwrap();
    let identifier = "a".repeat(1024);
    let mut checksum = 0u64;
    let start = Instant::now();
    for i in 0..iterations {
        let value = match mode {
            "parse" => match Version::parse(black_box(versions[i as usize % versions.len()])) {
                Ok(v) => v.major.wrapping_add(v.minor).wrapping_add(v.patch).wrapping_add(v.pre.len() as u64),
                Err(e) => { black_box(e); 17 },
            },
            "req" => {
                let r = VersionReq::parse(black_box(requirements[i as usize % requirements.len()])).unwrap();
                (black_box(&r).matches(black_box(&version)) as u64) + r.comparators.len() as u64
            },
            "identifier" => {
                let p = Prerelease::new(black_box(identifier.as_str())).unwrap();
                let c = black_box(&p).clone();
                black_box(c.as_str()).len() as u64 + (black_box(&p) == black_box(&c)) as u64
            },
            _ => panic!("unknown workload"),
        };
        checksum = checksum.wrapping_add(black_box(value));
    }
    let elapsed = start.elapsed().as_nanos();
    println!("{{\"workload\":\"{}\",\"iterations\":{},\"checksum\":{},\"elapsed_ns\":{}}}", mode, iterations, black_box(checksum), elapsed);
}
