//! `compare_bytes` reaches its one-byte path and memcmp with operands MIR
//! moves; both paths must read the original pointers and length.

#[inline(never)]
pub fn bytes_equal(a: &[u8], b: &[u8]) -> bool {
    a == b
}

#[inline(never)]
pub fn bytes_order(a: &[u8], b: &[u8]) -> core::cmp::Ordering {
    a.cmp(b)
}
