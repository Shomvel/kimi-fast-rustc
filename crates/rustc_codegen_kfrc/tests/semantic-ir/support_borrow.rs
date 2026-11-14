#[inline(never)]
pub fn supported(x: u64) -> u64 {
    let pair = (x, 7u64);
    pair.0
}
#[inline(never)]
pub fn requires_address(x: u64) -> u64 {
    let mut value = (x, 9u64);
    let address = &mut value;
    address.0 += 1;
    address.0
}
