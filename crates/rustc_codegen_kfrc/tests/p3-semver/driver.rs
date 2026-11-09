//! A visible workload alongside the unchanged upstream integration tests.
use semver::{BuildMetadata, Prerelease, Version, VersionReq};

fn main() {
    let versions = [
        "0.0.0",
        "1.2.3",
        "1.2.3-alpha.1+build.5678",
        "2.0.0-rc.1",
        "18446744073709551615.1.0",
        "1.2.3-abcdefghijklmnop.123456789+long-build-metadata",
        "",
        "01.2.3",
        "1.2",
        "1.2.3-01",
        "1.2.3+",
        "18446744073709551616.0.0",
    ];
    for text in versions {
        match Version::parse(text) {
            Ok(value) => {
                let cloned = value.clone();
                println!(
                    "version {text:?}: {value} debug={value:?} clone={}",
                    value == cloned
                );
            }
            Err(error) => println!("version {text:?}: error {error}"),
        }
    }
    for requirement in [
        "*",
        "^1.2.3",
        ">=1.0.0, <2.0.0",
        "~1.2",
        "1.*",
        "=2.0.0-rc.1",
        ">=",
    ] {
        match VersionReq::parse(requirement) {
            Ok(value) => {
                let matches: Vec<_> = ["0.1.0", "1.2.3", "1.9.0", "2.0.0-rc.1", "2.0.0"]
                    .iter()
                    .map(|text| value.matches(&Version::parse(text).unwrap()))
                    .collect();
                println!("requirement {requirement:?}: {value} debug={value:?} {matches:?}");
            }
            Err(error) => println!("requirement {requirement:?}: error {error}"),
        }
    }
    for length in [0, 1, 7, 8, 9, 127, 128, 1024] {
        let text = "a".repeat(length);
        let pre = Prerelease::new(&text).unwrap();
        let build = BuildMetadata::new(&text).unwrap();
        println!(
            "identifier {length}: {} {} {} {}",
            pre.len(),
            build.len(),
            pre == pre.clone(),
            build == build.clone()
        );
    }
}
