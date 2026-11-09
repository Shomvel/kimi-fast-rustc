extern crate emission_reachability;

fn main() {
    for value in [0u64, 1, 8, 9, 100, u64::MAX] {
        let n = value % 9;
        let expected = (value ^ 0x5a)
            ^ value.wrapping_mul(3).wrapping_add(7)
            ^ (1 + n * (n + 1) / 2);
        let actual = emission_reachability::evaluate(value);
        assert_eq!(actual, expected);
        println!("{value} {actual}");
    }
}
