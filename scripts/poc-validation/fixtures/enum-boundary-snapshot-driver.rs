extern crate enum_boundary_snapshot;
use enum_boundary_snapshot::*;
fn main() {
    for selector in 0..4 {
        for value in [0, 1, 255, u64::MAX] {
            let expected = match selector {
                0 => value,
                1 => 17,
                2 => value as u8 as u64,
                _ => value ^ 3 ^ 19 ^ 71,
            };
            let (sum, original) = call_result(selector, value);
            assert_eq!(sum, expected);
            let again = match original {
                Ok(n) => n,
                Err(e) => checksum(e),
            };
            assert_eq!(again, expected);
            let (sum, original) = parameter(original);
            assert_eq!(sum, expected);
            assert_eq!(
                match original {
                    Ok(n) => n,
                    Err(e) => checksum(e),
                },
                expected
            );
        }
    }
    println!("enum boundary snapshots: 16 cases passed");
}
