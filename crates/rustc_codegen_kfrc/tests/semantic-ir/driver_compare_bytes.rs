use semantic_values::compare_bytes_probe::*;

pub fn check() {
    let inputs: [&[u8]; 7] = [b"", b"a", b"b", b"aaaaaaaaa", b"bbbbbbbbb", b"aaaaaaaab", b"aaaaaaaaaaaaaaaa"];
    let mut equal = 0u32;
    for a in inputs {
        for b in inputs {
            assert_eq!(bytes_equal(a, b), a == b);
            assert_eq!(bytes_order(a, b), a.cmp(b));
            equal += u32::from(bytes_equal(a, b));
        }
    }
    assert_eq!(equal, inputs.len() as u32);
    println!("compare-bytes {equal}");
}
