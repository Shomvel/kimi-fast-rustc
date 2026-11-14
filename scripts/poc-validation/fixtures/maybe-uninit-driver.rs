extern crate maybe_uninit_values;
use maybe_uninit_values::*;

fn main() {
    // Returning and copying uninitialized storage is legal; never read as T.
    let opaque = opaque_uninitialized();
    std::hint::black_box(opaque);
    assert_eq!(invalid_bool_storage(), 2);
    for value in [0u64, 1, 255, u64::MAX] {
        assert_eq!(initialized_roundtrip(value as u32), value as u32);
        assert_eq!(
            partial_initialization(value as u8, value),
            (value as u8 as u64) ^ value
        );
        assert_eq!(empty_and_singleton(value), value);
        let values = [value, value.wrapping_add(1), 13, 27];
        let expected = (values[0] ^ values[3])
            .wrapping_add(values[1] ^ values[3])
            .wrapping_add(values[2])
            ^ values[2]
            ^ (values[1] ^ values[3]);
        let actual = array_iterator(values);
        assert_eq!(actual, expected);
        println!("{value} {actual}");
    }
}
