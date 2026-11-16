use semantic_values::constant_stores_probe::*;

pub fn check() {
    for seed in 0..=u8::MAX {
        let actual = constant_template(seed);
        let mut expected = *b"0123456789abcdefghijklmnopqrstu";
        expected[0] = seed;
        expected[30] = seed.wrapping_add(1);
        assert_eq!(actual, expected);
        assert_eq!(
            packed_fields(seed),
            (seed, [0, 255, 1, 254, 2, 253, 3], seed.wrapping_add(1))
        );
    }
    assert_eq!(observed_initialization(), 108);
    println!("constant-stores seeds=256 template=31 fields=7 observed=108");
}
