//! semver 1.0.28 driver. `run` keeps the per-iteration bodies of
//! scripts/poc-validation/semver-perf-driver.rs; `corpus` prints a deterministic differential.
use semver::{BuildMetadata, Comparator, Prerelease, Version, VersionReq};
use std::{hint::black_box, time::Instant};

const VERSIONS: [&str; 6] = ["1.2.3", "0.0.0", "12.34.56-alpha.7+build.123", "18446744073709551615.0.1", "1.02.3", "2.3.4-rc.1"];
const REQUIREMENTS: [&str; 6] = ["^1.2.3", ">=1.0, <3", "~2.3", "*", "1.2.x", ">=0.0.1-alpha"];

fn run(mode: &str, iterations: u64) -> u64 {
    let version = Version::parse("1.2.3-alpha.7").unwrap();
    let identifier = "a".repeat(1024);
    let mut checksum = 0u64;
    for i in 0..iterations {
        let value = match mode {
            "parse" => match Version::parse(black_box(VERSIONS[i as usize % VERSIONS.len()])) {
                Ok(v) => v.major.wrapping_add(v.minor).wrapping_add(v.patch).wrapping_add(v.pre.len() as u64),
                Err(e) => { black_box(e); 17 }
            },
            "req" => {
                let r = VersionReq::parse(black_box(REQUIREMENTS[i as usize % REQUIREMENTS.len()])).unwrap();
                (black_box(&r).matches(black_box(&version)) as u64) + r.comparators.len() as u64
            }
            "identifier" => {
                let p = Prerelease::new(black_box(identifier.as_str())).unwrap();
                let c = black_box(&p).clone();
                black_box(c.as_str()).len() as u64 + (black_box(&p) == black_box(&c)) as u64
            }
            _ => panic!("unknown workload {mode}"),
        };
        checksum = checksum.wrapping_add(black_box(value));
    }
    checksum
}

fn corpus() {
    let versions = [
        "1.2.3", "0.0.0", "12.34.56-alpha.7+build.123", "18446744073709551615.0.1", "18446744073709551616.0.0",
        "1.02.3", "2.3.4-rc.1", "1.0.0-alpha", "1.0.0-alpha.1", "1.0.0-alpha.beta", "1.0.0-beta", "1.0.0-beta.2",
        "1.0.0-beta.11", "1.0.0-rc.1", "1.0.0", "1.0.0+20130313144700", "1.0.0-0.3.7", "1.0.0-x.7.z.92",
        "1.0.0-x-y-z.--", "", "1", "1.2", "1.2.3.4", "v1.2.3", "1.2.3-", "1.2.3+", "1.2.3-01", "1.2.3-a..b",
        " 1.2.3", "1.2.3 ", "1.2.3-αβ", "0.0.1-a.b.c.d.e.f.g.h.i.j.k.l.m.n.o.p+meta.data.here",
    ];
    let mut parsed = Vec::new();
    for s in versions {
        match Version::parse(s) {
            Ok(v) => {
                println!("v {s:?} ok {v} {v:?} pre={} build={}", v.pre.as_str(), v.build.as_str());
                parsed.push(v);
            }
            Err(e) => println!("v {s:?} err {e}"),
        }
    }
    for a in &parsed {
        let row: String = parsed.iter().map(|b| match a.cmp(b) { std::cmp::Ordering::Less => '<', std::cmp::Ordering::Equal => '=', std::cmp::Ordering::Greater => '>' }).collect();
        println!("cmp {a} {row}");
    }
    let reqs = [
        "^1.2.3", ">=1.0, <3", "~2.3", "*", "1.2.x", ">=0.0.1-alpha", "=1.0.0", "<1.0.0-beta", "^0.0.1", "^0.1",
        "~1", "1.*.*", ">= 1.2, < 1.5", "", "^", ">=1.2.3,", "1.2.3 - 2.3.4", "~>1.2", "<=0.0.0", ">1.0.0-alpha.1",
        "^1.2.3-beta.2", "=0.0.0-0", "*, *", "1.x, >0.1",
    ];
    for s in reqs {
        match VersionReq::parse(s) {
            Ok(r) => {
                let m: String = parsed.iter().map(|v| if r.matches(v) { '1' } else { '0' }).collect();
                println!("r {s:?} ok {r} n={} {m}", r.comparators.len());
            }
            Err(e) => println!("r {s:?} err {e}"),
        }
    }
    for s in [">=1.2.3", "^0", "~0.1.2-rc", "=1.2.3+build", "1.2.3"] {
        match Comparator::parse(s) {
            Ok(c) => println!("c {s:?} ok {c} {:?}", c.op),
            Err(e) => println!("c {s:?} err {e}"),
        }
    }
    for s in ["", "alpha", "alpha.1", "0", "01", "a-b", "a..b", "-x"] {
        println!("p {s:?} {:?} b {:?}", Prerelease::new(s).map(|p| p.to_string()).map_err(|e| e.to_string()),
                 BuildMetadata::new(s).map(|p| p.to_string()).map_err(|e| e.to_string()));
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
