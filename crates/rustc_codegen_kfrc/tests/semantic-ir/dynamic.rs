//! A real LLVM vtable crosses into the sole semantic backend for dispatch/drop.
pub trait Value {
    fn read(&self, bias: u64) -> u64;
}

#[inline(never)]
pub fn call(value: &dyn Value, bias: u64) -> u64 {
    value.read(bias)
}

#[inline(never)]
pub fn size(value: &dyn Value) -> usize {
    core::mem::size_of_val(value)
}

#[inline(never)]
pub fn alignment(value: &dyn Value) -> usize {
    core::mem::align_of_val(value)
}

#[inline(never)]
pub fn destroy(value: Box<dyn Value>) {
    drop(value);
}
