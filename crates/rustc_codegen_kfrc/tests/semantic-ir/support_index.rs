#[inline(never)]
pub fn supported(x: u64) -> u64 {
    let pair = (x, 7u64);
    pair.0
}
#[inline(never)]
pub fn dynamic_index(values: [u64; 3], index: usize) -> u64 {
    values[index]
}
