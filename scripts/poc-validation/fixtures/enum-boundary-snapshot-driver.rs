extern crate enum_boundary_snapshot;
use enum_boundary_snapshot::*;
fn main() {
    for text in ["", "2026-11-10T01:23:45.123456789Z", "µs", "中文🌍"] {
        let actual = utf8_expect(text.as_bytes());
        assert_eq!(actual, text);
        assert_eq!(actual.as_ptr(), text.as_ptr());
        assert_eq!(utf8_result(text.as_bytes()), Ok(text));
    }
    for bytes in [&b"\xff"[..], &b"a\xe4\xb8"[..], &b"\xc0\xaf"[..]] {
        assert_eq!(utf8_result(bytes), std::str::from_utf8(bytes));
    }
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
