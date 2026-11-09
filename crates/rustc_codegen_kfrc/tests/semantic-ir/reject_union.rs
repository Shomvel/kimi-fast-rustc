pub union OutsideScope {
    pub x: u64,
    pub y: f64,
}
#[inline(never)]
pub unsafe fn union_value(x: OutsideScope) -> u64 {
    x.x
}
