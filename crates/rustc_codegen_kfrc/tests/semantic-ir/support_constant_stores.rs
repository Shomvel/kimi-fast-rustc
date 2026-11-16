#[inline(never)]
pub fn constant_template(seed: u8) -> [u8; 31] {
    let mut output = *b"0123456789abcdefghijklmnopqrstu";
    output[0] = seed;
    output[30] = seed.wrapping_add(1);
    output
}

#[inline(never)]
pub fn observed_initialization() -> u8 {
    let mut output = [17u8, 29, 43];
    let first = core::hint::black_box(&output)[0];
    output[0] = 91;
    first.wrapping_add(core::hint::black_box(&output)[0])
}

#[inline(never)]
pub fn packed_fields(seed: u8) -> (u8, [u8; 7], u8) {
    (seed, [0, 255, 1, 254, 2, 253, 3], seed.wrapping_add(1))
}
