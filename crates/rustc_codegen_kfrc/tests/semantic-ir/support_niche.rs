#[inline(never)]
pub fn supported(x: u64) -> u64 {
    let pair = (x, 7u64);
    pair.0
}
#[inline(never)]
pub fn relay(value: Option<&u64>) -> Option<&u64> {
    value
}
