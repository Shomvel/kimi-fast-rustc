#[repr(u8)]
pub enum DynamicVariant {
    Empty = 3,
    Data(u64) = 9,
}
#[inline(never)]
pub fn supported(x: u64) -> u64 {
    let pair = (x, 7u64);
    pair.0
}
#[inline(never)]
pub fn decode_argument(value: DynamicVariant) -> u64 {
    match value {
        DynamicVariant::Empty => 71,
        DynamicVariant::Data(x) => x,
    }
}
