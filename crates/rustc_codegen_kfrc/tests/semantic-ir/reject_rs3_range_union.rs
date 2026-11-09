//! Ordinary unions remain outside S's supported value subset. Keep this out
//! of the positive driver module even when the driver would not call it.
#[repr(C)]
pub union Mixed {
    pub restricted: core::num::NonZeroU32,
    pub raw: u32,
}

#[inline(never)]
pub fn raw_union(value: Mixed) -> bool {
    unsafe { value.raw != 0 }
}
