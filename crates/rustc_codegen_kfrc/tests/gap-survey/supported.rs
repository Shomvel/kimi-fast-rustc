#[unsafe(no_mangle)]
pub fn supported(value: usize) -> usize {
    value.wrapping_add(1)
}
