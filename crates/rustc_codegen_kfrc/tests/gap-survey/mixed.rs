use std::sync::atomic::{AtomicUsize, Ordering};

#[unsafe(no_mangle)]
pub fn supported(value: usize) -> usize {
    value.wrapping_add(1)
}

#[unsafe(no_mangle)]
pub fn first_atomic(value: &AtomicUsize) -> usize {
    value.load(Ordering::Relaxed)
}

#[unsafe(no_mangle)]
pub fn second_atomic(value: &AtomicUsize) -> usize {
    value.fetch_sub(1, Ordering::AcqRel)
}

#[unsafe(no_mangle)]
pub fn floating(value: f64) -> f64 {
    value + 1.0
}

#[unsafe(no_mangle)]
pub fn assembly() {
    unsafe {
        std::arch::asm!("nop");
    }
}
